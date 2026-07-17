//! PCI device information from /sys/bus/pci/devices.
//!
//! Shows PCI devices with their BDF addresses, vendor/device IDs, class codes,
//! and bound driver information. This is what you want when you SSH into a
//! machine and realize lspci isn't installed.
//!
//! We don't do PCI ID database lookups (that would require external files),
//! so you'll see "0x10de" instead of "NVIDIA Corporation". The hex IDs are
//! actually more useful for scripting anyway.

use crate::cli::GlobalOptions;
use crate::fields::pci as f;
use crate::filter::{matches_any, opt_str};
use crate::io;
use crate::json::{begin_kv_output_streaming, StreamingJsonWriter};
use crate::print::{self, TextWriter};
use crate::stack::StackString;

const PCI_SYSFS_PATH: &str = "/sys/bus/pci/devices";

/// Information about a PCI device.
pub struct PciDevice {
    /// Bus:Device.Function address (e.g., "0000:01:00.0")
    pub bdf: StackString<16>,
    /// Vendor ID
    pub vendor_id: u16,
    /// Device ID
    pub device_id: u16,
    /// Class code (3 bytes: class, subclass, prog-if)
    pub class: u32,
    /// Subsystem vendor ID (optional)
    pub subsystem_vendor_id: Option<u16>,
    /// Subsystem device ID (optional)
    pub subsystem_device_id: Option<u16>,
    /// Revision ID
    pub revision: Option<u8>,
    /// Currently bound driver (if any)
    pub driver: Option<StackString<64>>,
    /// NUMA node (for NUMA systems)
    pub numa_node: Option<i32>,
    /// IOMMU group number
    pub iommu_group: Option<u32>,
    /// Is this a bridge?
    pub is_bridge: bool,
    /// Device enabled?
    pub enabled: Option<bool>,
    /// D-state (power state)
    pub d_state: Option<StackString<16>>,
}

impl PciDevice {
    /// Read a PCI device from sysfs.
    pub fn read(bdf: &str) -> Option<Self> {
        // Build base path
        let base: StackString<64> = io::join_path(PCI_SYSFS_PATH, bdf);

        // Must have at least vendor and device
        let vendor_path: StackString<128> = io::join_path(base.as_str(), "vendor");
        let device_path: StackString<128> = io::join_path(base.as_str(), "device");
        let class_path: StackString<128> = io::join_path(base.as_str(), "class");

        let vendor_id: u16 = io::read_file_hex(vendor_path.as_str())?;
        let device_id: u16 = io::read_file_hex(device_path.as_str())?;
        let class: u32 = io::read_file_hex(class_path.as_str()).unwrap_or(0);

        // Subsystem IDs
        let subsys_vendor_path: StackString<128> = io::join_path(base.as_str(), "subsystem_vendor");
        let subsys_device_path: StackString<128> = io::join_path(base.as_str(), "subsystem_device");
        let subsystem_vendor_id: Option<u16> = io::read_file_hex(subsys_vendor_path.as_str());
        let subsystem_device_id: Option<u16> = io::read_file_hex(subsys_device_path.as_str());

        // Revision
        let revision_path: StackString<128> = io::join_path(base.as_str(), "revision");
        let revision: Option<u8> = io::read_file_hex(revision_path.as_str());

        // Driver is a symlink - we want just the name
        let driver_path: StackString<128> = io::join_path(base.as_str(), "driver");
        let driver: Option<StackString<64>> = io::read_symlink_name(driver_path.as_str());

        // NUMA node might be -1 (no NUMA) or a node number
        let numa_path: StackString<128> = io::join_path(base.as_str(), "numa_node");
        let numa_node: Option<i32> = io::read_file_parse(numa_path.as_str());

        // IOMMU group is a symlink, we extract the group number from the path
        let iommu_path: StackString<128> = io::join_path(base.as_str(), "iommu_group");
        let iommu_group: Option<u32> = io::read_symlink_name::<16>(iommu_path.as_str())
            .and_then(|s| s.as_str().parse().ok());

        // Bridge detection: class code 0x06xxxx
        let is_bridge = (class >> 16) == 0x06;

        // Enabled state
        let enable_path: StackString<128> = io::join_path(base.as_str(), "enable");
        let enabled: Option<bool> = io::read_file_parse::<u8>(enable_path.as_str()).map(|v| v != 0);

        // Power state (D0, D3hot, etc.)
        let power_path: StackString<128> = io::join_path(base.as_str(), "power_state");
        let d_state: Option<StackString<16>> = io::read_file_stack(power_path.as_str());

        Some(PciDevice {
            bdf: StackString::from_str(bdf),
            vendor_id,
            device_id,
            class,
            subsystem_vendor_id,
            subsystem_device_id,
            revision,
            driver,
            numa_node,
            iommu_group,
            is_bridge,
            enabled,
            d_state,
        })
    }

    /// Check if this device matches the filter pattern.
    fn matches_filter(&self, pattern: &str, case_insensitive: bool) -> bool {
        let vendor_hex = io::format_hex_u16(self.vendor_id);
        let device_hex = io::format_hex_u16(self.device_id);
        let driver_str = opt_str(&self.driver);
        let fields = [
            self.bdf.as_str(),
            driver_str,
            vendor_hex.as_str(),
            device_hex.as_str(),
        ];
        matches_any(&fields, pattern, case_insensitive)
    }

    /// Output as text.
    fn print_text(&self, verbose: bool) {
        let mut w = TextWriter::new();

        w.field_str(f::BDF, self.bdf.as_str());
        w.field_str(f::VENDOR_ID, io::format_hex_u16(self.vendor_id).as_str());
        w.field_str(f::DEVICE_ID, io::format_hex_u16(self.device_id).as_str());
        w.field_str(f::CLASS, io::format_hex_class(self.class).as_str());

        if let Some(ref driver) = self.driver {
            w.field_str(f::DRIVER, driver.as_str());
        }

        if verbose {
            if let Some(v) = self.subsystem_vendor_id {
                w.field_str(f::SUBSYS_VENDOR, io::format_hex_u16(v).as_str());
            }
            if let Some(v) = self.subsystem_device_id {
                w.field_str(f::SUBSYS_DEVICE, io::format_hex_u16(v).as_str());
            }
            if let Some(v) = self.revision {
                w.field_str(f::REVISION, io::format_hex_u8(v).as_str());
            }
            if let Some(v) = self.numa_node {
                w.field_i64(f::NUMA_NODE, v as i64);
            }
            if let Some(v) = self.iommu_group {
                w.field_u64(f::IOMMU_GROUP, v as u64);
            }
            if let Some(v) = self.enabled {
                w.field_u64(f::ENABLED, if v { 1 } else { 0 });
            }
            if let Some(ref state) = self.d_state {
                w.field_str(f::POWER_STATE, state.as_str());
            }
        }

        w.finish();
    }

    /// Write as JSON object.
    fn write_json(&self, w: &mut StreamingJsonWriter, verbose: bool) {
        w.array_object_begin();

        w.field_str(f::BDF, self.bdf.as_str());
        w.field_str(f::VENDOR_ID, io::format_hex_u16(self.vendor_id).as_str());
        w.field_str(f::DEVICE_ID, io::format_hex_u16(self.device_id).as_str());
        w.field_str(f::CLASS, io::format_hex_class(self.class).as_str());
        w.field_str_opt(f::DRIVER, self.driver.as_ref().map(|s| s.as_str()));

        if verbose {
            if let Some(v) = self.subsystem_vendor_id {
                w.field_str(f::SUBSYS_VENDOR, io::format_hex_u16(v).as_str());
            }
            if let Some(v) = self.subsystem_device_id {
                w.field_str(f::SUBSYS_DEVICE, io::format_hex_u16(v).as_str());
            }
            if let Some(v) = self.revision {
                w.field_str(f::REVISION, io::format_hex_u8(v).as_str());
            }
            if let Some(v) = self.numa_node {
                w.field_i64(f::NUMA_NODE, v as i64);
            }
            if let Some(v) = self.iommu_group {
                w.field_u64(f::IOMMU_GROUP, v as u64);
            }
            if let Some(v) = self.enabled {
                w.field_bool(f::ENABLED, v);
            }
            w.field_str_opt(f::POWER_STATE, self.d_state.as_ref().map(|s| s.as_str()));
            w.field_bool(f::IS_BRIDGE, self.is_bridge);
        }

        w.array_object_end();
    }
}

/// Entry point for `kv pci` subcommand.
pub fn run(opts: &GlobalOptions) -> i32 {
    if !io::path_exists(PCI_SYSFS_PATH) {
        if opts.json {
            let mut w = begin_kv_output_streaming(opts.pretty, "pci");
            w.field_array("data");
            w.end_field_array();
            w.end_object();
            w.finish();
        } else {
            print::println("pci: no PCI bus found");
        }
        return 0;
    }

    let filter = opts.filter.as_ref().map(|s| s.as_str());
    let case_insensitive = opts.filter_case_insensitive;

    if opts.json {
        let mut w = begin_kv_output_streaming(opts.pretty, "pci");
        w.field_array("data");

        let mut count = 0;
        io::for_each_dir_entry(PCI_SYSFS_PATH, |bdf| {
            if let Some(dev) = PciDevice::read(bdf) {
                if let Some(pattern) = filter {
                    if !dev.matches_filter(pattern, case_insensitive) {
                        return;
                    }
                }
                dev.write_json(&mut w, opts.verbose);
                count += 1;
            }
        });

        w.end_field_array();
        w.end_object();
        w.finish();
    } else {
        let mut count = 0;
        io::for_each_dir_entry(PCI_SYSFS_PATH, |bdf| {
            if let Some(dev) = PciDevice::read(bdf) {
                if let Some(pattern) = filter {
                    if !dev.matches_filter(pattern, case_insensitive) {
                        return;
                    }
                }
                dev.print_text(opts.verbose);
                count += 1;
            }
        });

        if count == 0 {
            if filter.is_some() {
                print::println("pci: no matching devices");
            } else {
                print::println("pci: no PCI devices found");
            }
        }
    }

    0
}

/// Write PCI devices to JSON writer (for snapshot).
#[cfg(feature = "snapshot")]
pub fn write_snapshot(w: &mut StreamingJsonWriter, verbose: bool) {
    if !io::path_exists(PCI_SYSFS_PATH) {
        return;
    }

    w.key("pci");
    w.begin_array();
    io::for_each_dir_entry(PCI_SYSFS_PATH, |bdf| {
        if let Some(dev) = PciDevice::read(bdf) {
            dev.write_json(w, verbose);
        }
    });
    w.end_array();
}
