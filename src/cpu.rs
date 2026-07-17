//! CPU information from /proc/cpuinfo and /sys/devices/system/cpu.
//!
//! Parsing /proc/cpuinfo is a bit of an adventure because the format varies
//! between architectures:
//!   - x86: "model name", "vendor_id", "flags" (huge list), "cpu MHz"
//!   - ARM: "CPU implementer", "CPU part", "Features", "Hardware"
//!   - RISC-V: "isa" (e.g., rv64imafdvcsu), "mmu" (e.g., sv39)
//!
//! We do our best to provide useful information regardless of architecture,
//! but some fields may be missing on some platforms. That's life in embedded.

use crate::cli::GlobalOptions;
use crate::fields::cpu as f;
use crate::io;
use crate::json::StreamingJsonWriter;
use crate::print;
use crate::stack::StackString;

const CPUINFO_PATH: &str = "/proc/cpuinfo";

/// Maximum unique physical/core IDs we track for topology detection.
const MAX_IDS: usize = 64;

/// Simple set for tracking unique u32 values (replaces HashSet).
struct IdSet {
    ids: [u32; MAX_IDS],
    count: usize,
}

impl IdSet {
    const fn new() -> Self {
        Self {
            ids: [0; MAX_IDS],
            count: 0,
        }
    }

    fn insert(&mut self, id: u32) {
        // Check if already present
        for i in 0..self.count {
            if self.ids[i] == id {
                return;
            }
        }
        // Add if space available
        if self.count < MAX_IDS {
            self.ids[self.count] = id;
            self.count += 1;
        }
    }

    fn len(&self) -> usize {
        self.count
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// CPU information structure.
#[derive(Default)]
pub struct CpuInfo {
    /// Number of logical CPUs (threads)
    pub logical_cpus: u32,
    /// Model name (x86) or CPU part description
    pub model_name: Option<StackString<128>>,
    /// Vendor ID (GenuineIntel, AuthenticAMD, ARM, etc.)
    pub vendor_id: Option<StackString<64>>,
    /// CPU family (x86)
    pub cpu_family: Option<u32>,
    /// Model number (x86)
    pub model: Option<u32>,
    /// Stepping (x86)
    pub stepping: Option<u32>,
    /// MHz (may vary per core, we take the first) - stored as fixed point (mhz * 100)
    pub cpu_mhz_x100: Option<u32>,
    /// Cache size (x86, usually L2 or L3)
    pub cache_size: Option<StackString<32>>,
    /// Number of physical cores per socket
    pub cores_per_socket: Option<u32>,
    /// Number of sockets (physical packages)
    pub sockets: Option<u32>,
    /// Architecture (from uname or inferred)
    pub architecture: Option<StackString<16>>,
    /// RISC-V ISA string (e.g., "rv64imafdvcsu")
    pub isa: Option<StackString<64>>,
    /// RISC-V MMU type (e.g., "sv39")
    pub mmu: Option<StackString<16>>,
}

impl CpuInfo {
    /// Read CPU information from /proc/cpuinfo.
    pub fn read() -> Option<Self> {
        let contents: StackString<65536> = io::read_file_stack(CPUINFO_PATH)?;
        Some(Self::parse(contents.as_str()))
    }

    /// Parse /proc/cpuinfo content.
    pub fn parse(content: &str) -> Self {
        let mut info = CpuInfo::default();
        let mut logical_cpus = 0u32;
        let mut physical_ids = IdSet::new();
        let mut core_ids = IdSet::new();

        let mut first_cpu = true;
        let mut current_block_has_processor = false;

        for line in content.lines() {
            if line.trim().is_empty() {
                if current_block_has_processor {
                    logical_cpus += 1;
                    first_cpu = false;
                }
                current_block_has_processor = false;
                continue;
            }

            let Some((key, value)) = parse_cpuinfo_line(line) else {
                continue;
            };

            if key == "processor" {
                current_block_has_processor = true;
            }

            if key == "physical id" {
                if let Ok(id) = value.parse::<u32>() {
                    physical_ids.insert(id);
                }
            }
            if key == "core id" {
                if let Ok(id) = value.parse::<u32>() {
                    core_ids.insert(id);
                }
            }

            // Hardware line appears outside CPU blocks on ARM
            if key == "Hardware" {
                info.model_name = Some(StackString::from_str(value));
                continue;
            }

            if first_cpu || info.model_name.is_none() {
                match key {
                    "model name" => info.model_name = Some(StackString::from_str(value)),
                    "vendor_id" => info.vendor_id = Some(StackString::from_str(value)),
                    "cpu family" => info.cpu_family = value.parse().ok(),
                    "model" => info.model = value.parse().ok(),
                    "stepping" => info.stepping = value.parse().ok(),
                    "cpu MHz" => {
                        // Parse as fixed point to avoid float
                        if let Some(mhz) = parse_mhz(value) {
                            info.cpu_mhz_x100 = Some(mhz);
                        }
                    }
                    "cache size" => info.cache_size = Some(StackString::from_str(value)),
                    // ARM-specific
                    "CPU implementer" => {
                        if info.vendor_id.is_none() {
                            let mut s = StackString::from_str("ARM (");
                            s.push_str(value);
                            s.push(')');
                            info.vendor_id = Some(s);
                        }
                    }
                    "CPU part" => {
                        if info.model_name.is_none() {
                            let mut s = StackString::from_str("ARM Part ");
                            s.push_str(value);
                            info.model_name = Some(s);
                        }
                    }
                    // RISC-V specific
                    "isa" => info.isa = Some(StackString::from_str(value)),
                    "mmu" => info.mmu = Some(StackString::from_str(value)),
                    _ => {}
                }
            }
        }

        if current_block_has_processor {
            logical_cpus += 1;
        }

        info.logical_cpus = logical_cpus;

        if !physical_ids.is_empty() {
            info.sockets = Some(physical_ids.len() as u32);
            if !core_ids.is_empty() {
                info.cores_per_socket = Some(core_ids.len() as u32);
            }
        }

        info.architecture = detect_architecture();
        info
    }

    /// Output as text (KEY=VALUE format).
    pub fn print_text(&self, verbose: bool) {
        let mut w = print::TextWriter::new();

        w.field_u64(f::LOGICAL_CPUS, self.logical_cpus as u64);
        w.field_quoted_opt(f::MODEL_NAME, self.model_name.as_ref().map(|s| s.as_str()));
        w.field_str_opt(f::VENDOR_ID, self.vendor_id.as_ref().map(|s| s.as_str()));
        w.field_u64_opt(f::SOCKETS, self.sockets.map(|v| v as u64));
        w.field_u64_opt(f::CORES_PER_SOCKET, self.cores_per_socket.map(|v| v as u64));

        // RISC-V specific
        w.field_str_opt(f::ISA, self.isa.as_ref().map(|s| s.as_str()));
        w.field_str_opt(f::MMU, self.mmu.as_ref().map(|s| s.as_str()));

        if verbose {
            w.field_u64_opt(f::CPU_FAMILY, self.cpu_family.map(|v| v as u64));
            w.field_u64_opt(f::MODEL, self.model.map(|v| v as u64));
            w.field_u64_opt(f::STEPPING, self.stepping.map(|v| v as u64));
            if let Some(mhz_x100) = self.cpu_mhz_x100 {
                w.field_mhz(f::CPU_MHZ, mhz_x100);
            }
            w.field_quoted_opt(f::CACHE_SIZE, self.cache_size.as_ref().map(|s| s.as_str()));
            w.field_str_opt(f::ARCHITECTURE, self.architecture.as_ref().map(|s| s.as_str()));
        }

        w.finish();
    }

    /// Output as JSON.
    pub fn print_json(&self, pretty: bool, verbose: bool) {
        let mut w = StreamingJsonWriter::new(pretty);

        w.begin_object();
        w.field_str("kv_version", env!("CARGO_PKG_VERSION"));
        w.field_str("subcommand", "cpu");

        w.field_object("data");
        w.field_u64(f::LOGICAL_CPUS, self.logical_cpus as u64);
        w.field_str_opt(f::MODEL_NAME, self.model_name.as_ref().map(|s| s.as_str()));
        w.field_str_opt(f::VENDOR_ID, self.vendor_id.as_ref().map(|s| s.as_str()));
        w.field_u64_opt(f::SOCKETS, self.sockets.map(|v| v as u64));
        w.field_u64_opt(f::CORES_PER_SOCKET, self.cores_per_socket.map(|v| v as u64));
        w.field_str_opt(f::ISA, self.isa.as_ref().map(|s| s.as_str()));
        w.field_str_opt(f::MMU, self.mmu.as_ref().map(|s| s.as_str()));

        if verbose {
            w.field_u64_opt(f::CPU_FAMILY, self.cpu_family.map(|v| v as u64));
            w.field_u64_opt(f::MODEL, self.model.map(|v| v as u64));
            w.field_u64_opt(f::STEPPING, self.stepping.map(|v| v as u64));
            if let Some(mhz_x100) = self.cpu_mhz_x100 {
                let mut buf: StackString<16> = StackString::new();
                format_mhz_into(&mut buf, mhz_x100);
                w.field_str(f::CPU_MHZ, buf.as_str());
            }
            w.field_str_opt(f::CACHE_SIZE, self.cache_size.as_ref().map(|s| s.as_str()));
            w.field_str_opt(f::ARCHITECTURE, self.architecture.as_ref().map(|s| s.as_str()));
        }

        w.end_field_object();
        w.end_object();
        w.finish();
    }
}

/// Parse a single line from /proc/cpuinfo.
fn parse_cpuinfo_line(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    Some((key.trim(), value.trim()))
}

/// Parse MHz value as fixed point (x100) to avoid floats.
fn parse_mhz(s: &str) -> Option<u32> {
    // Parse "3191.998" as 319199
    let mut result: u32 = 0;
    let mut decimal_places = 0;
    let mut seen_dot = false;

    for c in s.bytes() {
        if c == b'.' {
            seen_dot = true;
            continue;
        }
        if c.is_ascii_digit() {
            if seen_dot {
                if decimal_places >= 2 {
                    continue; // Ignore extra decimal places
                }
                decimal_places += 1;
            }
            result = result.checked_mul(10)?.checked_add((c - b'0') as u32)?;
        }
    }

    // Pad to 2 decimal places
    while decimal_places < 2 {
        result = result.checked_mul(10)?;
        decimal_places += 1;
    }

    Some(result)
}

/// Format MHz into a StackString.
fn format_mhz_into(buf: &mut StackString<16>, mhz_x100: u32) {
    let whole = mhz_x100 / 100;
    let frac = mhz_x100 % 100;
    let mut itoa_buf = itoa::Buffer::new();
    buf.push_str(itoa_buf.format(whole));
    buf.push('.');
    if frac < 10 {
        buf.push('0');
    }
    buf.push_str(itoa_buf.format(frac));
}

/// Try to detect the CPU architecture.
fn detect_architecture() -> Option<StackString<16>> {
    #[cfg(target_arch = "x86_64")]
    return Some(StackString::from_str("x86_64"));

    #[cfg(target_arch = "x86")]
    return Some(StackString::from_str("x86"));

    #[cfg(target_arch = "aarch64")]
    return Some(StackString::from_str("aarch64"));

    #[cfg(target_arch = "arm")]
    return Some(StackString::from_str("arm"));

    #[cfg(target_arch = "riscv64")]
    return Some(StackString::from_str("riscv64"));

    #[cfg(target_arch = "powerpc64")]
    return Some(StackString::from_str("powerpc64"));

    #[cfg(target_arch = "mips")]
    return Some(StackString::from_str("mips"));

    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "riscv64",
        target_arch = "powerpc64",
        target_arch = "mips"
    )))]
    None
}

/// Entry point for `kv cpu` subcommand.
pub fn run(opts: &GlobalOptions) -> i32 {
    let Some(info) = CpuInfo::read() else {
        if opts.json {
            let mut w = StreamingJsonWriter::new(opts.pretty);
            w.begin_object();
            w.field_str("kv_version", env!("CARGO_PKG_VERSION"));
            w.field_str("subcommand", "cpu");
            w.key("data");
            w.value_null();
            w.field_str("error", "cannot read /proc/cpuinfo");
            w.end_object();
            w.finish();
        } else {
            print::println("cpu: cannot read /proc/cpuinfo");
        }
        return 0;
    };

    if opts.json {
        info.print_json(opts.pretty, opts.verbose);
    } else {
        info.print_text(opts.verbose);
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_counts_all_processor_blocks() {
        // Regression guard for the 4 KiB read truncation: a 22-thread x86
        // cpuinfo is ~25 KiB and must still count every block.
        let mut content = std::string::String::new();
        for i in 0..22 {
            content.push_str(&std::format!(
                "processor\t: {}\nvendor_id\t: GenuineIntel\nmodel name\t: Test CPU\n\n",
                i
            ));
        }
        let info = CpuInfo::parse(&content);
        assert_eq!(info.logical_cpus, 22);
    }

    #[test]
    fn parse_sockets_and_cores() {
        let mut content = std::string::String::new();
        for i in 0..4 {
            content.push_str(&std::format!(
                "processor\t: {}\nphysical id\t: 0\ncore id\t: {}\n\n",
                i,
                i / 2
            ));
        }
        let info = CpuInfo::parse(&content);
        assert_eq!(info.logical_cpus, 4);
        assert_eq!(info.sockets, Some(1));
        assert_eq!(info.cores_per_socket, Some(2));
    }

    #[test]
    fn parse_arm_hardware_line() {
        let content = "processor\t: 0\nCPU implementer\t: 0x41\n\nHardware\t: BCM2835\n";
        let info = CpuInfo::parse(content);
        assert_eq!(info.logical_cpus, 1);
        assert_eq!(info.model_name.as_deref(), Some("BCM2835"));
    }
}
