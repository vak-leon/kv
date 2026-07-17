//! Block device information from /sys/block.
//!
//! Provides an lsblk-like view of block devices and their partitions.
//! Reads from /sys/block for device info and cross-references with
//! /proc/self/mounts to show mount points.
//!
//! We handle the somewhat odd sysfs layout where partitions can appear
//! either as subdirectories of /sys/block/<disk>/ or as separate entries.

use crate::cli::GlobalOptions;
use crate::fields::block as f;
use crate::filter::{matches_any, opt_str};
use crate::io;
use crate::json::{begin_kv_output_streaming, StreamingJsonWriter};
use crate::print::{self, TextWriter};
use crate::stack::StackString;

const BLOCK_SYSFS_PATH: &str = "/sys/block";
const MOUNTS_PATH: &str = "/proc/self/mounts";

/// Maximum number of mount entries we track.
const MAX_MOUNT_ENTRIES: usize = 128;

/// Type of block device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    Disk,
    Part,
    Loop,
    Ram,
    Other,
}

impl BlockType {
    fn as_str(&self) -> &'static str {
        match self {
            BlockType::Disk => "disk",
            BlockType::Part => "part",
            BlockType::Loop => "loop",
            BlockType::Ram => "ram",
            BlockType::Other => "other",
        }
    }
}

/// Stack-based mountpoint lookup table.
/// Maps device paths (like "/dev/sda1") to mount points (like "/mnt/data").
struct MountpointMap {
    /// Device path -> mount point pairs.
    entries: [(StackString<64>, StackString<256>); MAX_MOUNT_ENTRIES],
    count: usize,
}

impl MountpointMap {
    /// Create an empty mountpoint map.
    fn new() -> Self {
        Self {
            entries: core::array::from_fn(|_| (StackString::new(), StackString::new())),
            count: 0,
        }
    }

    /// Add an entry to the map.
    fn insert(&mut self, device: &str, mountpoint: &str) {
        if self.count < MAX_MOUNT_ENTRIES {
            self.entries[self.count].0 = StackString::from_str(device);
            self.entries[self.count].1 = StackString::from_str(mountpoint);
            self.count += 1;
        }
    }

    /// Look up a device's mount point.
    fn get(&self, device: &str) -> Option<&str> {
        for i in 0..self.count {
            if self.entries[i].0.as_str() == device {
                return Some(self.entries[i].1.as_str());
            }
        }
        None
    }

    /// Read mount points from /proc/self/mounts.
    fn from_mounts() -> Self {
        let mut map = Self::new();

        let contents: Option<StackString<32768>> = io::read_file_stack(MOUNTS_PATH);
        let Some(contents) = contents else {
            return map;
        };

        for line in contents.as_str().lines() {
            let mut parts = line.split_whitespace();
            if let (Some(device), Some(mountpoint)) = (parts.next(), parts.next()) {
                // Only track /dev/* devices
                if device.starts_with("/dev/") {
                    map.insert(device, mountpoint);
                }
            }
        }

        map
    }
}

/// Information about a block device or partition.
pub struct BlockDevice {
    /// Device name (e.g., "sda", "sda1", "nvme0n1p1")
    pub name: StackString<32>,
    /// Device type
    pub dev_type: BlockType,
    /// Major device number
    pub major: u32,
    /// Minor device number
    pub minor: u32,
    /// Size in sectors
    pub size_sectors: u64,
    /// Sector size in bytes (usually 512)
    pub sector_size: u32,
    /// Is the device removable?
    pub removable: bool,
    /// Is the device read-only?
    pub ro: bool,
    /// Parent device name (for partitions)
    pub parent: Option<StackString<32>>,
    /// Mount point (if mounted)
    pub mountpoint: Option<StackString<256>>,
    /// Device model (for disks, if available)
    pub model: Option<StackString<64>>,
    /// Rotational device (HDD) or not (SSD)?
    pub rotational: Option<bool>,
    /// Scheduler in use
    pub scheduler: Option<StackString<32>>,
}

impl BlockDevice {
    /// Read a block device from sysfs.
    ///
    /// For partitions, most attributes (removable, queue/*, model) don't exist -
    /// they inherit physical characteristics from the parent disk. We skip reading
    /// them to avoid noisy debug output.
    fn read(name: &str, parent: Option<&str>, mountpoints: &MountpointMap) -> Option<Self> {
        let base: StackString<128> = if let Some(p) = parent {
            let parent_path: StackString<64> = io::join_path(BLOCK_SYSFS_PATH, p);
            io::join_path(parent_path.as_str(), name)
        } else {
            io::join_path(BLOCK_SYSFS_PATH, name)
        };

        // Must have at least size and dev (major:minor)
        let size_path: StackString<256> = io::join_path(base.as_str(), "size");
        if !io::path_exists(size_path.as_str()) {
            return None;
        }

        let size_sectors: u64 = io::read_file_parse(size_path.as_str()).unwrap_or(0);
        let dev_path: StackString<256> = io::join_path(base.as_str(), "dev");
        let dev_str: Option<StackString<16>> = io::read_file_stack(dev_path.as_str());
        let (major, minor) = parse_dev(dev_str.as_ref()?.as_str())?;

        // Determine device type
        let is_partition = parent.is_some();
        let dev_type = if is_partition {
            BlockType::Part
        } else if name.starts_with("loop") {
            BlockType::Loop
        } else if name.starts_with("ram") {
            BlockType::Ram
        } else {
            BlockType::Disk
        };

        // For partitions, skip reading disk-level attributes that don't exist
        // (removable, queue/*, device/model). They inherit from parent.
        let (removable, sector_size, model, rotational, scheduler) = if is_partition {
            (false, 512, None, None, None)
        } else {
            let removable_path: StackString<256> = io::join_path(base.as_str(), "removable");
            let removable = io::read_file_parse::<u8>(removable_path.as_str())
                .map(|v| v != 0)
                .unwrap_or(false);

            // Sector size - try hw_sector_size first, fall back to logical
            let hw_sector_path: StackString<256> = io::join_path(base.as_str(), "queue/hw_sector_size");
            let logical_path: StackString<256> = io::join_path(base.as_str(), "queue/logical_block_size");
            let sector_size = io::read_file_parse(hw_sector_path.as_str())
                .or_else(|| io::read_file_parse(logical_path.as_str()))
                .unwrap_or(512);

            // Model - try device/model first (SCSI/NVMe), then device/name (MMC/SD)
            let model_path: StackString<256> = io::join_path(base.as_str(), "device/model");
            let name_path: StackString<256> = io::join_path(base.as_str(), "device/name");
            let model: Option<StackString<64>> = io::read_file_stack(model_path.as_str())
                .or_else(|| io::read_file_stack(name_path.as_str()));

            // Rotational flag
            let rot_path: StackString<256> = io::join_path(base.as_str(), "queue/rotational");
            let rotational = io::read_file_parse::<u8>(rot_path.as_str())
                .map(|v| v != 0);

            // Scheduler (e.g., "[mq-deadline] none" - extract the active one)
            let sched_path: StackString<256> = io::join_path(base.as_str(), "queue/scheduler");
            let scheduler: Option<StackString<32>> = io::read_file_stack::<64>(sched_path.as_str())
                .and_then(|s| extract_active_scheduler(s.as_str()));

            (removable, sector_size, model, rotational, scheduler)
        };

        // ro is valid for both disks and partitions
        let ro_path: StackString<256> = io::join_path(base.as_str(), "ro");
        let ro = io::read_file_parse::<u8>(ro_path.as_str())
            .map(|v| v != 0)
            .unwrap_or(false);

        // Look up mount point by device path
        let mut dev_path_buf: StackString<64> = StackString::new();
        dev_path_buf.push_str("/dev/");
        dev_path_buf.push_str(name);
        let mountpoint = mountpoints.get(dev_path_buf.as_str())
            .map(StackString::from_str);

        Some(BlockDevice {
            name: StackString::from_str(name),
            dev_type,
            major,
            minor,
            size_sectors,
            sector_size,
            removable,
            ro,
            parent: parent.map(StackString::from_str),
            mountpoint,
            model,
            rotational,
            scheduler,
        })
    }

    /// Check if this device matches the filter pattern.
    fn matches_filter(&self, pattern: &str, case_insensitive: bool) -> bool {
        let fields = [
            self.name.as_str(),
            opt_str(&self.model),
            opt_str(&self.mountpoint),
            self.dev_type.as_str(),
        ];
        matches_any(&fields, pattern, case_insensitive)
    }

    /// Output as text.
    fn print_text(&self, verbose: bool, human: bool) {
        let mut w = TextWriter::new();

        w.field_str(f::NAME, self.name.as_str());
        w.field_str(f::TYPE, self.dev_type.as_str());

        // majmin as "8:0"
        let mut majmin: StackString<16> = StackString::new();
        let mut buf = itoa::Buffer::new();
        majmin.push_str(buf.format(self.major));
        majmin.push(':');
        majmin.push_str(buf.format(self.minor));
        w.field_str(f::MAJMIN, majmin.as_str());

        if human {
            let size = io::format_sectors_human(self.size_sectors, self.sector_size);
            w.field_str(f::SIZE, size.as_str());
        } else {
            w.field_u64(f::SIZE_SECTORS, self.size_sectors);
        }

        if let Some(ref parent) = self.parent {
            w.field_str(f::PARENT, parent.as_str());
        }

        if let Some(ref mp) = self.mountpoint {
            w.field_quoted(f::MOUNTPOINT, mp.as_str());
        }

        if verbose {
            if !human {
                w.field_u64(f::SECTOR_SIZE, self.sector_size as u64);
            }
            w.field_u64(f::REMOVABLE, if self.removable { 1 } else { 0 });
            w.field_u64(f::RO, if self.ro { 1 } else { 0 });

            if let Some(ref model) = self.model {
                w.field_quoted(f::MODEL, model.as_str());
            }
            if let Some(rot) = self.rotational {
                w.field_u64(f::ROTATIONAL, if rot { 1 } else { 0 });
            }
            if let Some(ref sched) = self.scheduler {
                w.field_str(f::SCHEDULER, sched.as_str());
            }
        }

        w.finish();
    }

    /// Write as JSON object.
    fn write_json(&self, w: &mut StreamingJsonWriter, verbose: bool, human: bool) {
        w.array_object_begin();

        w.field_str(f::NAME, self.name.as_str());
        w.field_str(f::TYPE, self.dev_type.as_str());
        w.field_u64(f::MAJOR, self.major as u64);
        w.field_u64(f::MINOR, self.minor as u64);

        if human {
            let size = io::format_sectors_human(self.size_sectors, self.sector_size);
            w.field_str(f::SIZE, size.as_str());
        } else {
            w.field_u64(f::SIZE_SECTORS, self.size_sectors);
        }

        w.field_str_opt(f::PARENT, self.parent.as_ref().map(|s| s.as_str()));
        w.field_str_opt(f::MOUNTPOINT, self.mountpoint.as_ref().map(|s| s.as_str()));

        if verbose {
            if !human {
                w.field_u64(f::SECTOR_SIZE, self.sector_size as u64);
            }
            w.field_bool(f::REMOVABLE, self.removable);
            w.field_bool(f::RO, self.ro);
            w.field_str_opt(f::MODEL, self.model.as_ref().map(|s| s.as_str()));
            if let Some(rot) = self.rotational {
                w.field_bool(f::ROTATIONAL, rot);
            }
            w.field_str_opt(f::SCHEDULER, self.scheduler.as_ref().map(|s| s.as_str()));
        }

        w.array_object_end();
    }
}

/// Parse major:minor string.
fn parse_dev(s: &str) -> Option<(u32, u32)> {
    let (maj, min) = s.split_once(':')?;
    Some((maj.trim().parse().ok()?, min.trim().parse().ok()?))
}

/// Extract active scheduler from scheduler file content.
/// Format: "mq-deadline kyber [none]" -> "none"
fn extract_active_scheduler(s: &str) -> Option<StackString<32>> {
    // Active scheduler is in brackets
    let start = s.find('[')?;
    let end = s.find(']')?;
    if start < end {
        Some(StackString::from_str(&s[start + 1..end]))
    } else {
        None
    }
}

/// Entry point for `kv block` subcommand.
pub fn run(opts: &GlobalOptions) -> i32 {
    if !io::path_exists(BLOCK_SYSFS_PATH) {
        if opts.json {
            let mut w = begin_kv_output_streaming(opts.pretty, "block");
            w.field_array("data");
            w.end_field_array();
            w.end_object();
            w.finish();
        } else {
            print::println("block: no block devices found");
        }
        return 0;
    }

    let mountpoints = MountpointMap::from_mounts();
    let filter = opts.filter.as_ref().map(|s| s.as_str());
    let case_insensitive = opts.filter_case_insensitive;

    if opts.json {
        let mut w = begin_kv_output_streaming(opts.pretty, "block");
        w.field_array("data");

        let mut count = 0;
        io::for_each_dir_entry(BLOCK_SYSFS_PATH, |disk_name| {
            if let Some(disk) = BlockDevice::read(disk_name, None, &mountpoints) {
                // Skip loop devices with size 0 (unbound)
                if disk.dev_type == BlockType::Loop && disk.size_sectors == 0 {
                    return;
                }

                // Output disk if it matches filter (or no filter)
                if let Some(pattern) = filter {
                    if disk.matches_filter(pattern, case_insensitive) {
                        disk.write_json(&mut w, opts.verbose, opts.human);
                        count += 1;
                    }
                } else {
                    disk.write_json(&mut w, opts.verbose, opts.human);
                    count += 1;
                }

                // Look for partitions as subdirectories
                let disk_path: StackString<64> = io::join_path(BLOCK_SYSFS_PATH, disk_name);
                io::for_each_dir_entry(disk_path.as_str(), |entry_name| {
                    // Partition directories start with the disk name
                    if entry_name.starts_with(disk_name) {
                        if let Some(part) = BlockDevice::read(entry_name, Some(disk_name), &mountpoints) {
                            if let Some(pattern) = filter {
                                if part.matches_filter(pattern, case_insensitive) {
                                    part.write_json(&mut w, opts.verbose, opts.human);
                                    count += 1;
                                }
                            } else {
                                part.write_json(&mut w, opts.verbose, opts.human);
                                count += 1;
                            }
                        }
                    }
                });
            }
        });

        w.end_field_array();
        w.end_object();
        w.finish();
    } else {
        let mut count = 0;
        io::for_each_dir_entry(BLOCK_SYSFS_PATH, |disk_name| {
            if let Some(disk) = BlockDevice::read(disk_name, None, &mountpoints) {
                // Skip loop devices with size 0 (unbound)
                if disk.dev_type == BlockType::Loop && disk.size_sectors == 0 {
                    return;
                }

                // Output disk if it matches filter (or no filter)
                if let Some(pattern) = filter {
                    if disk.matches_filter(pattern, case_insensitive) {
                        disk.print_text(opts.verbose, opts.human);
                        count += 1;
                    }
                } else {
                    disk.print_text(opts.verbose, opts.human);
                    count += 1;
                }

                // Look for partitions as subdirectories
                let disk_path: StackString<64> = io::join_path(BLOCK_SYSFS_PATH, disk_name);
                io::for_each_dir_entry(disk_path.as_str(), |entry_name| {
                    // Partition directories start with the disk name
                    if entry_name.starts_with(disk_name) {
                        if let Some(part) = BlockDevice::read(entry_name, Some(disk_name), &mountpoints) {
                            if let Some(pattern) = filter {
                                if part.matches_filter(pattern, case_insensitive) {
                                    part.print_text(opts.verbose, opts.human);
                                    count += 1;
                                }
                            } else {
                                part.print_text(opts.verbose, opts.human);
                                count += 1;
                            }
                        }
                    }
                });
            }
        });

        if count == 0 {
            if filter.is_some() {
                print::println("block: no matching devices");
            } else {
                print::println("block: no block devices found");
            }
        }
    }

    0
}

/// Write block devices to JSON writer (for snapshot).
#[cfg(feature = "snapshot")]
pub fn write_snapshot(w: &mut StreamingJsonWriter, verbose: bool) {
    if !io::path_exists(BLOCK_SYSFS_PATH) {
        return;
    }

    let mountpoints = MountpointMap::from_mounts();

    w.key("block");
    w.begin_array();
    io::for_each_dir_entry(BLOCK_SYSFS_PATH, |disk_name| {
        if let Some(disk) = BlockDevice::read(disk_name, None, &mountpoints) {
            // Skip loop devices with size 0 (unbound)
            if disk.dev_type == BlockType::Loop && disk.size_sectors == 0 {
                return;
            }

            disk.write_json(w, verbose, false);

            // Look for partitions as subdirectories
            let disk_path: StackString<64> = io::join_path(BLOCK_SYSFS_PATH, disk_name);
            io::for_each_dir_entry(disk_path.as_str(), |entry_name| {
                // Partition directories start with the disk name
                if entry_name.starts_with(disk_name) {
                    if let Some(part) = BlockDevice::read(entry_name, Some(disk_name), &mountpoints) {
                        part.write_json(w, verbose, false);
                    }
                }
            });
        }
    });
    w.end_array();
}
