//! kv - Kernel View
//!
//! Library half of kv: all the actual logic lives here so unit tests can
//! run it on the host with std. The binary entry point (main.rs) stays
//! no_std and just dispatches into these modules.
//!
//! no_std in release; std is allowed under `cfg(test)` so tests can use
//! tempfiles, String, etc.

#![cfg_attr(not(test), no_std)]

pub mod cli;
pub mod debug;
pub mod fields;
pub mod filter;
pub mod io;
pub mod json;
pub mod print;
pub mod stack;

// Subcommand modules - conditionally compiled based on features.

#[cfg(feature = "mem")]
pub mod mem;

#[cfg(feature = "pci")]
pub mod pci;
#[cfg(feature = "usb")]
pub mod usb;
#[cfg(feature = "block")]
pub mod block;
#[cfg(feature = "net")]
pub mod net;
#[cfg(feature = "cpu")]
pub mod cpu;
#[cfg(feature = "mounts")]
pub mod mounts;
#[cfg(feature = "thermal")]
pub mod thermal;
#[cfg(feature = "power")]
pub mod power;
#[cfg(feature = "snapshot")]
pub mod snapshot;

#[cfg(all(
    feature = "dt",
    any(target_arch = "arm", target_arch = "aarch64", target_arch = "riscv64", target_arch = "powerpc64", target_arch = "mips")
))]
pub mod dt;

/// Stub for architectures where a devicetree is not typically present.
#[cfg(all(
    feature = "dt",
    not(any(target_arch = "arm", target_arch = "aarch64", target_arch = "riscv64", target_arch = "powerpc64", target_arch = "mips"))
))]
pub mod dt {
    pub fn run(_opts: &crate::cli::GlobalOptions, _args: &crate::cli::ExtraArgs) -> i32 {
        crate::print::println("dt: devicetree not typically available on this architecture");
        0
    }
}
