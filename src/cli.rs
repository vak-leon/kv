//! Command-line argument parsing for kv.
//!
//! We roll our own because external crates are forbidden, and honestly,
//! our needs are simple enough that clap would be overkill anyway.
//!
//! The basic pattern:
//! - First positional arg (after program name) is the subcommand
//! - Everything else is flags/options for that subcommand
//! - Global flags like --json, --pretty, --verbose apply to all subcommands
//!
//! This version uses stack-based storage to avoid heap allocation.

use core::ffi::{c_char, CStr};
use crate::print;
use crate::stack::StackString;

// =============================================================================
// Input Safety Limits
// =============================================================================

/// Maximum length for filter patterns (defense against memory exhaustion).
/// 1024 chars is plenty for any reasonable substring match.
const MAX_FILTER_LEN: usize = 1024;

/// Maximum length for subcommand name.
const MAX_SUBCMD_LEN: usize = 32;

/// Maximum number of extra arguments to store.
const MAX_EXTRA_ARGS: usize = 8;

/// Maximum length for each extra argument.
const MAX_ARG_LEN: usize = 256;

/// Type alias for filter string.
pub type FilterStr = StackString<MAX_FILTER_LEN>;

/// Type alias for subcommand string.
pub type SubcmdStr = StackString<MAX_SUBCMD_LEN>;

/// Type alias for argument string.
pub type ArgStr = StackString<MAX_ARG_LEN>;

/// Global options that apply to all subcommands.
#[derive(Clone, Default)]
pub struct GlobalOptions {
    /// Output as JSON instead of text
    pub json: bool,
    /// Pretty-print JSON (only meaningful with json=true)
    pub pretty: bool,
    /// Verbose output - show extra fields
    pub verbose: bool,
    /// Human-readable output (e.g., "1.5G" instead of bytes)
    pub human: bool,
    /// Show help
    pub help: bool,
    /// Filter pattern (from -f, or -F which also lowercases it)
    pub filter: Option<FilterStr>,
    /// Whether filter is case-insensitive (-F vs -f)
    pub filter_case_insensitive: bool,
    /// Debug mode - show file access and parse errors
    pub debug: bool,
}

/// Arguments storage - fixed-size array of stack strings.
pub struct ExtraArgs {
    args: [ArgStr; MAX_EXTRA_ARGS],
    count: usize,
}

impl ExtraArgs {
    /// Create empty args storage.
    pub const fn new() -> Self {
        Self {
            args: [
                StackString::new(), StackString::new(),
                StackString::new(), StackString::new(),
                StackString::new(), StackString::new(),
                StackString::new(), StackString::new(),
            ],
            count: 0,
        }
    }

    /// Add an argument (ignores if full).
    pub fn push(&mut self, arg: &str) {
        if self.count < MAX_EXTRA_ARGS {
            self.args[self.count] = StackString::from_str(arg);
            self.count += 1;
        }
    }

    /// Get the number of arguments.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Get first argument.
    pub fn first(&self) -> Option<&str> {
        if self.count > 0 {
            Some(self.args[0].as_str())
        } else {
            None
        }
    }

    /// Iterate over arguments.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.args[..self.count].iter().map(|s| s.as_str())
    }
}

impl Default for ExtraArgs {
    fn default() -> Self {
        Self::new()
    }
}

/// The parsed command-line invocation.
pub struct Invocation {
    /// The subcommand to run (pci, usb, block, etc.), if any
    pub subcommand: Option<SubcmdStr>,
    /// Global options
    pub options: GlobalOptions,
    /// Remaining arguments for the subcommand
    pub args: ExtraArgs,
}

/// Check the raw environment block for KV_DEBUG being set to a truthy value
/// (anything but empty or "0"). We get envp straight from origin - there is
/// no libc getenv here.
///
/// # Safety
/// `envp` must be a valid NULL-terminated array of C strings (origin
/// guarantees this for the pointer it passes to origin_main).
pub unsafe fn env_has_kv_debug(envp: *const *const u8) -> bool {
    if envp.is_null() {
        return false;
    }
    let mut i = 0;
    loop {
        // SAFETY: caller guarantees envp is NULL-terminated
        let entry = unsafe { *envp.offset(i) };
        if entry.is_null() {
            return false;
        }
        // SAFETY: caller guarantees entries are valid C strings
        let bytes = unsafe { CStr::from_ptr(entry as *const c_char) }.to_bytes();
        if let Some(value) = bytes.strip_prefix(b"KV_DEBUG=") {
            return !value.is_empty() && value != b"0";
        }
        i += 1;
    }
}

/// Warn on stderr that a -f/-F pattern was cut at MAX_FILTER_LEN bytes.
fn warn_filter_truncated() {
    print::eprint("Warning: filter truncated to ");
    let mut buf = itoa::Buffer::new();
    print::eprint(buf.format(MAX_FILTER_LEN));
    print::eprintln(" bytes");
}

impl Invocation {
    /// Parse command-line arguments into an Invocation from raw argc/argv.
    ///
    /// # Safety
    /// `argv` must be a valid pointer to an array of at least `argc` valid C strings.
    pub unsafe fn parse_from_raw(argc: i32, argv: *const *const u8) -> Self {
        // Process arguments directly without intermediate Vec allocation
        let mut opts = GlobalOptions::default();
        let mut subcommand: Option<SubcmdStr> = None;
        let mut extra_args = ExtraArgs::new();
        let mut skip_next = false;
        let mut found_subcommand = false;

        // Skip program name (i=0), start from i=1
        for i in 1..argc as isize {
            if skip_next {
                skip_next = false;
                continue;
            }

            // SAFETY: caller guarantees argv is valid array of C strings
            let arg_ptr = unsafe { *argv.offset(i) };
            let cstr = unsafe { CStr::from_ptr(arg_ptr as *const c_char) };
            let arg = match cstr.to_str() {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Handle special top-level flags first
            if !found_subcommand {
                match arg {
                    "--help" | "-H" => {
                        opts.help = true;
                        subcommand = Some(StackString::from_str(arg));
                        found_subcommand = true;
                        continue;
                    }
                    "--version" | "-V" => {
                        subcommand = Some(StackString::from_str(arg));
                        found_subcommand = true;
                        continue;
                    }
                    "help" => {
                        opts.help = true;
                        subcommand = Some(StackString::from_str("help"));
                        found_subcommand = true;
                        continue;
                    }
                    _ => {}
                }
            }

            // Check for flags
            if arg.starts_with('-') {
                match arg {
                    "-j" | "--json" => opts.json = true,
                    "-p" | "--pretty" => opts.pretty = true,
                    "-v" | "--verbose" => opts.verbose = true,
                    "-h" | "--human" => opts.human = true,
                    "-H" | "--help" => opts.help = true,
                    "-D" | "--debug" => opts.debug = true,
                    "-f" | "--filter" => {
                        // Next arg is the filter pattern
                        if i + 1 < argc as isize {
                            let next_ptr = unsafe { *argv.offset(i + 1) };
                            let next_cstr = unsafe { CStr::from_ptr(next_ptr as *const c_char) };
                            if let Ok(pattern) = next_cstr.to_str() {
                                let mut filter = FilterStr::new();
                                if !filter.push_str(pattern) {
                                    warn_filter_truncated();
                                }
                                opts.filter = Some(filter);
                                opts.filter_case_insensitive = false;
                                skip_next = true;
                            }
                        }
                    }
                    "-F" | "--ifilter" => {
                        // Next arg is the filter pattern (case-insensitive).
                        // ASCII-lowercased here, and matching folds ASCII only.
                        if i + 1 < argc as isize {
                            let next_ptr = unsafe { *argv.offset(i + 1) };
                            let next_cstr = unsafe { CStr::from_ptr(next_ptr as *const c_char) };
                            if let Ok(pattern) = next_cstr.to_str() {
                                let mut filter = FilterStr::new();
                                for c in pattern.chars() {
                                    if !filter.push(c.to_ascii_lowercase()) {
                                        warn_filter_truncated();
                                        break;
                                    }
                                }
                                opts.filter = Some(filter);
                                opts.filter_case_insensitive = true;
                                skip_next = true;
                            }
                        }
                    }
                    // Combined short flags like -jpv
                    s if !s.starts_with("--") && s.len() > 2 => {
                        let has_filter = s.contains('f') || s.contains('F');
                        if !has_filter {
                            for c in s[1..].chars() {
                                match c {
                                    'j' => opts.json = true,
                                    'p' => opts.pretty = true,
                                    'v' => opts.verbose = true,
                                    'h' => opts.human = true,
                                    'H' => opts.help = true,
                                    'D' => opts.debug = true,
                                    _ => {
                                        // Unknown flag - treat as extra arg
                                        extra_args.push(arg);
                                        break;
                                    }
                                }
                            }
                        } else {
                            extra_args.push(arg);
                        }
                    }
                    _ => extra_args.push(arg),
                }
            } else if !found_subcommand {
                // First non-flag is subcommand
                subcommand = Some(StackString::from_str(arg));
                found_subcommand = true;
            } else {
                // Extra argument
                extra_args.push(arg);
            }
        }

        Invocation {
            subcommand,
            options: opts,
            args: extra_args,
        }
    }

    /// Check if help was requested (either via flag or "help" subcommand).
    pub fn wants_help(&self) -> bool {
        self.options.help || self.subcommand.as_ref().map(|s| s.as_str()) == Some("help")
    }

    /// Check if version was requested.
    pub fn wants_version(&self) -> bool {
        match self.subcommand.as_ref().map(|s| s.as_str()) {
            Some("--version") | Some("-V") => true,
            _ => false,
        }
    }

    /// Get the subcommand to show help for, if any.
    pub fn help_subject(&self) -> Option<&str> {
        // "kv help pci" - subject is in args
        if let Some(subcmd) = self.args.first() {
            return Some(subcmd);
        }
        // "kv pci -H" - subject is subcommand (unless it's "help" itself)
        if let Some(ref subcmd) = self.subcommand {
            let s = subcmd.as_str();
            if s != "help" && s != "--help" && s != "-H" {
                return Some(s);
            }
        }
        None
    }
}

/// Print the main help text.
pub fn print_help() {
    print::println(env!("CARGO_PKG_DESCRIPTION"));
    print::println_empty();
    print::print(concat!(
        "USAGE:\n",
        "    kv <SUBCOMMAND> [OPTIONS]\n",
        "\n",
        "OPTIONS:\n",
        "    -j, --json        Output as JSON\n",
        "    -p, --pretty      Pretty-print JSON (use with -j)\n",
        "    -v, --verbose     Show additional fields (most commands, see -H)\n",
        "    -h, --human       Human-readable sizes (1K, 2.5M, 3G)\n",
        "    -f <pattern>      Filter output (case-sensitive)\n",
        "    -F <pattern>      Filter output (case-insensitive)\n",
        "    -D, --debug       Show debug info on stderr (file access, dir scans)\n",
        "    -H, --help        Show help (use 'kv <cmd> -H' for subcommand details)\n",
        "    -V, --version     Show version and compiled features\n",
        "\n",
        "SUBCOMMANDS:\n",
    ));

    #[cfg(feature = "pci")]
    print::print("    pci        Show PCI devices\n");
    #[cfg(feature = "usb")]
    print::print("    usb        Show USB devices\n");
    #[cfg(feature = "block")]
    print::print("    block      Show block devices and partitions\n");
    #[cfg(feature = "net")]
    print::print("    net        Show network interfaces\n");
    #[cfg(feature = "cpu")]
    print::print("    cpu        Show CPU information\n");
    #[cfg(feature = "mem")]
    print::print("    mem        Show memory information\n");
    #[cfg(feature = "mounts")]
    print::print("    mounts     Show mounted filesystems\n");
    #[cfg(feature = "thermal")]
    print::print("    thermal    Show temperature sensors\n");
    #[cfg(feature = "power")]
    print::print("    power      Show power supplies/batteries\n");
    #[cfg(feature = "dt")]
    print::print("    dt         Show devicetree nodes (use -H for dt-specific options)\n");
    #[cfg(feature = "snapshot")]
    print::print("    snapshot   Combined JSON dump of all info\n");

    print::print(concat!(
        "\n",
        "ENVIRONMENT:\n",
        "    KV_DEBUG=1    Enable debug mode (same as -D)\n",
        "\n",
        "EXIT CODES:\n",
        "    0    Success (even if some data unavailable)\n",
        "    1    Error (bad arguments, severe I/O failure)\n",
        "\n",
        "EXAMPLES:\n",
        "    kv pci                # List PCI devices\n",
        "    kv pci -jph           # As pretty JSON with human-readable sizes\n",
        "    kv net -f wlP         # Network interfaces containing exactly 'wlP'\n",
        "    kv net -F up          # Same, case-insensitive\n",
        "    kv snapshot           # Everything, as JSON\n",
        "    KV_DEBUG=1 kv mem     # With debug output\n",
    ));
}

/// Print version information including compiled features.
pub fn print_version() {
    print::print("kv ");
    print::println(env!("CARGO_PKG_VERSION"));

    // Print features without Vec
    print::print("features:");
    let mut first = true;

    macro_rules! print_feature {
        ($name:expr) => {
            if first {
                print::print(" ");
                first = false;
            } else {
                print::print(", ");
            }
            print::print($name);
        };
    }

    #[cfg(feature = "pci")]
    print_feature!("pci");
    #[cfg(feature = "usb")]
    print_feature!("usb");
    #[cfg(feature = "block")]
    print_feature!("block");
    #[cfg(feature = "net")]
    print_feature!("net");
    #[cfg(feature = "cpu")]
    print_feature!("cpu");
    #[cfg(feature = "mem")]
    print_feature!("mem");
    #[cfg(feature = "mounts")]
    print_feature!("mounts");
    #[cfg(feature = "thermal")]
    print_feature!("thermal");
    #[cfg(feature = "power")]
    print_feature!("power");
    #[cfg(feature = "dt")]
    print_feature!("dt");
    #[cfg(feature = "snapshot")]
    print_feature!("snapshot");

    if first {
        print::print(" (none)");
    }
    print::println_empty();

    #[cfg(target_arch = "x86_64")]
    print::println("arch: x86_64");
    #[cfg(target_arch = "x86")]
    print::println("arch: x86");
    #[cfg(target_arch = "aarch64")]
    print::println("arch: aarch64");
    #[cfg(target_arch = "arm")]
    print::println("arch: arm");
    #[cfg(target_arch = "riscv64")]
    print::println("arch: riscv64");
    #[cfg(target_arch = "powerpc64")]
    print::println("arch: powerpc64");
    #[cfg(target_arch = "mips")]
    print::println("arch: mips");
}

/// Print help for a specific subcommand.
pub fn print_subcommand_help(subcommand: &str) {
    match subcommand {
        #[cfg(feature = "pci")]
        "pci" => print::print(concat!(
            "kv pci - Show PCI devices\n\n",
            "Reads PCI device information from /sys/bus/pci/devices/\n\n",
            "FIELDS (default):\n",
            "    bdf            Bus:Device.Function address\n",
            "    vendor_id      PCI vendor ID\n",
            "    device_id      PCI device ID\n",
            "    class          Device class code\n",
            "    driver         Bound driver name (if any)\n\n",
            "FIELDS (verbose):\n",
            "    subsystem_vendor_id, subsystem_device_id\n",
            "    numa_node, iommu_group\n",
        )),

        #[cfg(feature = "usb")]
        "usb" => print::print(concat!(
            "kv usb - Show USB devices\n\n",
            "Reads USB device information from /sys/bus/usb/devices/\n",
            "Filters out root hub entries for cleaner output.\n",
        )),

        #[cfg(feature = "block")]
        "block" => print::print(concat!(
            "kv block - Show block devices and partitions\n\n",
            "Reads block device information from /sys/block/\n",
            "Associates partitions with their parent disks.\n",
        )),

        #[cfg(feature = "net")]
        "net" => print::print(concat!(
            "kv net - Show network interfaces\n\n",
            "Reads network interface information from /sys/class/net/\n",
        )),

        #[cfg(feature = "cpu")]
        "cpu" => print::print(concat!(
            "kv cpu - Show CPU information\n\n",
            "Reads CPU information from /proc/cpuinfo and /sys/devices/system/cpu/\n",
        )),

        #[cfg(feature = "mem")]
        "mem" => print::print(concat!(
            "kv mem - Show memory information\n\n",
            "Reads memory information from /proc/meminfo\n\n",
            "FIELDS:\n",
            "    mem_total_kb      Total physical memory\n",
            "    mem_free_kb       Free memory\n",
            "    mem_available_kb  Available memory (free + reclaimable)\n",
            "    swap_total_kb     Total swap space\n",
            "    swap_free_kb      Free swap space\n",
        )),

        #[cfg(feature = "mounts")]
        "mounts" => print::print(concat!(
            "kv mounts - Show mounted filesystems\n\n",
            "Reads mount information from /proc/self/mounts\n",
        )),

        #[cfg(feature = "thermal")]
        "thermal" => print::print(concat!(
            "kv thermal - Show temperature sensors\n\n",
            "Reads thermal data from /sys/class/thermal/ (thermal zones)\n",
            "or /sys/class/hwmon/ (hardware monitors) as fallback.\n\n",
            "FIELDS:\n",
            "    sensor     Sensor type (cpu-thermal, coretemp, etc.)\n",
            "    label      Sensor label (Core 0, Package, etc.) - hwmon only\n",
            "    temp_c     Current temperature in Celsius\n\n",
            "FIELDS (verbose):\n",
            "    crit_c     Critical temperature threshold\n",
            "    policy     Thermal policy (step_wise, etc.)\n",
            "    source     Data source (thermal or hwmon)\n",
        )),

        #[cfg(feature = "power")]
        "power" => print::print(concat!(
            "kv power - Show power supplies and batteries\n\n",
            "Reads power supply info from /sys/class/power_supply/\n\n",
            "TYPES:\n",
            "    Battery    Laptop/device batteries\n",
            "    Mains      AC adapters\n",
            "    USB        USB power delivery sources\n",
            "    UPS        Uninterruptible power supplies\n\n",
            "FIELDS (verbose):\n",
            "    voltage_v, current_a, power_w\n",
        )),

        #[cfg(feature = "dt")]
        "dt" => print::print(concat!(
            "kv dt - Show devicetree nodes\n\n",
            "USAGE:\n",
            "    kv dt                  Show board model/compatible + node count\n",
            "    kv dt -v               List all nodes\n",
            "    kv dt /soc/uart@1000   Show specific node with all properties\n",
            "    kv dt -f <pattern>     Filter nodes by path or compatible\n",
            "    kv dt -d               Show only disabled nodes\n\n",
            "DT-SPECIFIC OPTIONS:\n",
            "    -d, --disabled      Show only nodes with status != okay\n\n",
            "Reads devicetree from /sys/firmware/devicetree/base/\n",
            "NOTE: Only available on systems with devicetree (ARM, RISC-V)\n",
        )),

        #[cfg(feature = "snapshot")]
        "snapshot" => print::print(concat!(
            "kv snapshot - Combined JSON dump\n\n",
            "Outputs all available system information as a single JSON object.\n",
            "Always outputs JSON (--json is implied).\n\n",
            "Use --pretty for human-readable formatting.\n",
        )),

        _ => {
            print::eprint("Unknown subcommand: ");
            print::eprintln(subcommand);
            print::eprintln("Run 'kv --help' for a list of subcommands.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::vec::Vec;

    /// Build a NULL-terminated argv (with "kv" as argv[0]) and parse it.
    fn parse(args: &[&str]) -> Invocation {
        let mut cstrings: Vec<CString> = Vec::with_capacity(args.len() + 1);
        cstrings.push(CString::new("kv").unwrap());
        for a in args {
            cstrings.push(CString::new(*a).unwrap());
        }
        let ptrs: Vec<*const u8> = cstrings.iter().map(|c| c.as_ptr() as *const u8).collect();
        // SAFETY: ptrs points at valid C strings for the duration of the call
        unsafe { Invocation::parse_from_raw(ptrs.len() as i32, ptrs.as_ptr()) }
    }

    #[test]
    fn subcommand_and_flags() {
        let inv = parse(&["pci", "-j", "-v"]);
        assert_eq!(inv.subcommand.as_deref(), Some("pci"));
        assert!(inv.options.json);
        assert!(inv.options.verbose);
        assert!(!inv.options.pretty);
    }

    #[test]
    fn flags_before_subcommand() {
        let inv = parse(&["-j", "cpu"]);
        assert_eq!(inv.subcommand.as_deref(), Some("cpu"));
        assert!(inv.options.json);
    }

    #[test]
    fn combined_short_flags() {
        let inv = parse(&["mem", "-jph"]);
        assert!(inv.options.json);
        assert!(inv.options.pretty);
        assert!(inv.options.human);
    }

    #[test]
    fn filter_takes_next_arg() {
        let inv = parse(&["net", "-f", "eth"]);
        assert_eq!(inv.options.filter.as_deref(), Some("eth"));
        assert!(!inv.options.filter_case_insensitive);
    }

    #[test]
    fn ifilter_lowercases_pattern() {
        let inv = parse(&["net", "-F", "WlP"]);
        assert_eq!(inv.options.filter.as_deref(), Some("wlp"));
        assert!(inv.options.filter_case_insensitive);
    }

    #[test]
    fn version_and_help_requests() {
        assert!(parse(&["--version"]).wants_version());
        assert!(parse(&["--help"]).wants_help());
        let inv = parse(&["help", "pci"]);
        assert!(inv.wants_help());
        assert_eq!(inv.help_subject(), Some("pci"));
    }

    #[test]
    fn env_kv_debug_detection() {
        fn check(entries: &[&[u8]]) -> bool {
            let cstrings: Vec<CString> =
                entries.iter().map(|e| CString::new(*e).unwrap()).collect();
            let mut ptrs: Vec<*const u8> =
                cstrings.iter().map(|c| c.as_ptr() as *const u8).collect();
            ptrs.push(core::ptr::null());
            // SAFETY: NULL-terminated array of valid C strings
            unsafe { env_has_kv_debug(ptrs.as_ptr()) }
        }
        assert!(check(&[b"PATH=/bin", b"KV_DEBUG=1"]));
        assert!(!check(&[b"PATH=/bin", b"KV_DEBUG=0"]));
        assert!(!check(&[b"PATH=/bin", b"KV_DEBUG="]));
        assert!(!check(&[b"PATH=/bin"]));
        assert!(!check(&[]));
    }
}
