//! kv - Kernel View
//!
//! Binary entry point: origin-based startup, panic handler, and subcommand
//! dispatch. All real logic lives in the library (lib.rs) so it can be
//! unit-tested on the host. See README.md for full documentation.

#![no_std]
#![no_main]

// Force link origin to get startup code and mem functions
extern crate origin;

use kv::cli::{Invocation, print_help, print_version, print_subcommand_help};
use kv::{debug, print};

/// Panic handler - minimal, just exits
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // In release builds, just exit immediately
    rustix::runtime::exit_group(101)
}

/// Dev builds (`cargo build`) link the prebuilt libcore, whose debug data
/// references this unwinding symbol. It can never be called with
/// panic=abort. Release builds rebuild core via build-std and drop it.
#[unsafe(no_mangle)]
extern "C" fn rust_eh_personality() {}

/// Entry point called by origin.
/// Origin calls this after performing program initialization.
#[unsafe(no_mangle)]
unsafe fn origin_main(argc: usize, argv: *mut *mut u8, envp: *mut *mut u8) -> i32 {
    // SAFETY: origin guarantees argc/argv/envp are valid
    let inv = unsafe { Invocation::parse_from_raw(argc as i32, argv as *const *const u8) };
    let env_debug = unsafe { kv::cli::env_has_kv_debug(envp as *const *const u8) };
    run(inv, env_debug)
}

fn run(inv: Invocation, env_debug: bool) -> i32 {
    debug::set_enabled(inv.options.debug || env_debug);

    kv::dbg_print!("kv ", env!("CARGO_PKG_VERSION"), " starting");
    if let Some(ref subcommand) = inv.subcommand {
        kv::dbg_print!("subcommand: ", subcommand.as_str());
    }

    // Handle version request
    if inv.wants_version() {
        print_version();
        return 0;
    }

    // Handle help request
    if inv.wants_help() {
        match inv.help_subject() {
            Some(subcmd) => print_subcommand_help(subcmd),
            None => print_help(),
        }
        return 0;
    }

    // No subcommand? Print usage and exit with error.
    let Some(ref subcommand) = inv.subcommand else {
        print::eprintln("Error: no subcommand specified");
        print::eprintln_empty();
        print::eprintln("Run 'kv --help' for usage information.");
        return 1;
    };

    // Dispatch to the appropriate subcommand.
    // Each match arm is conditionally compiled - if feature is off, it's not here.
    match subcommand.as_str() {
        #[cfg(feature = "pci")]
        "pci" => kv::pci::run(&inv.options),

        #[cfg(feature = "usb")]
        "usb" => kv::usb::run(&inv.options),

        #[cfg(feature = "block")]
        "block" => kv::block::run(&inv.options),

        #[cfg(feature = "net")]
        "net" => kv::net::run(&inv.options),

        #[cfg(feature = "cpu")]
        "cpu" => kv::cpu::run(&inv.options),

        #[cfg(feature = "mem")]
        "mem" => kv::mem::run(&inv.options),

        #[cfg(feature = "mounts")]
        "mounts" => kv::mounts::run(&inv.options),

        #[cfg(feature = "thermal")]
        "thermal" => kv::thermal::run(&inv.options),

        #[cfg(feature = "power")]
        "power" => kv::power::run(&inv.options),

        #[cfg(feature = "dt")]
        "dt" => kv::dt::run(&inv.options, &inv.args),

        #[cfg(feature = "snapshot")]
        "snapshot" => kv::snapshot::run(&inv.options),

        _unknown => {
            print::eprintln("Error: unknown subcommand");
            print::eprintln_empty();
            print::eprintln("Run 'kv --help' for a list of available subcommands.");
            1
        }
    }
}
