//! Debug output utilities for kv.
//!
//! When debug mode is enabled (via -D flag or KV_DEBUG=1 environment
//! variable), these helpers print diagnostic lines to stderr: which files
//! were read, which failed and with what errno, and how many entries each
//! directory scan found. Useful for troubleshooting on new/unusual hardware.
//!
//! Everything is built from plain string pushes - no core::fmt - so the
//! cost in binary size stays small.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::print;

/// Global debug mode flag, set once at startup.
static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable debug mode globally. Called once from main after parsing args.
pub fn set_enabled(enabled: bool) {
    DEBUG_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Check if debug mode is enabled.
#[inline]
pub fn is_enabled() -> bool {
    DEBUG_ENABLED.load(Ordering::Relaxed)
}

/// Print "[DEBUG] <parts...>" to stderr if debug mode is on.
pub fn msg(parts: &[&str]) {
    if !is_enabled() {
        return;
    }
    print::eprint("[DEBUG] ");
    for part in parts {
        print::eprint(part);
    }
    print::eprintln_empty();
}

/// Print "[DEBUG] FAIL <path>: errno <n>" to stderr if debug mode is on.
pub fn fail(path: &str, err: rustix::io::Errno) {
    if !is_enabled() {
        return;
    }
    print::eprint("[DEBUG] FAIL ");
    print::eprint(path);
    print::eprint(": errno ");
    let mut buf = itoa::Buffer::new();
    print::eprint(buf.format(err.raw_os_error()));
    print::eprintln_empty();
}

/// Print "[DEBUG] SCAN <path> (<n> entries)" to stderr if debug mode is on.
pub fn scan(path: &str, count: usize) {
    if !is_enabled() {
        return;
    }
    print::eprint("[DEBUG] SCAN ");
    print::eprint(path);
    print::eprint(" (");
    let mut buf = itoa::Buffer::new();
    print::eprint(buf.format(count));
    print::eprintln(" entries)");
}

/// Print a debug message to stderr if debug mode is enabled.
/// Takes string parts, not format args: dbg_print!("a", val, "b").
#[macro_export]
macro_rules! dbg_print {
    ($($part:expr),+ $(,)?) => {
        $crate::debug::msg(&[$($part),+])
    };
}

/// Print a debug message about a file read failure.
#[macro_export]
macro_rules! dbg_fail {
    ($path:expr, $err:expr) => {
        $crate::debug::fail($path, $err)
    };
}
