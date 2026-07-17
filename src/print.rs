//! Minimal print functions using rustix syscalls.
//!
//! These functions provide println!/eprintln!-like functionality without
//! pulling in core::fmt machinery. They're designed to be drop-in replacements
//! for the most common print patterns in kv.

use rustix::io::write;
use rustix::stdio::{stdout, stderr};

/// Print a string to stdout (no newline).
#[inline]
pub fn print(s: &str) {
    // SAFETY: stdout() is safe to call - it returns the process's stdout fd
    let _ = write(unsafe { stdout() }, s.as_bytes());
}

/// Print a string to stdout with newline.
#[inline]
pub fn println(s: &str) {
    // SAFETY: stdout() is safe to call - it returns the process's stdout fd
    let out = unsafe { stdout() };
    let _ = write(out, s.as_bytes());
    let _ = write(out, b"\n");
}

/// Print to stderr (no newline).
#[inline]
pub fn eprint(s: &str) {
    // SAFETY: stderr() is safe to call - it returns the process's stderr fd
    let _ = write(unsafe { stderr() }, s.as_bytes());
}

/// Print to stderr with newline.
#[inline]
pub fn eprintln(s: &str) {
    // SAFETY: stderr() is safe to call - it returns the process's stderr fd
    let err = unsafe { stderr() };
    let _ = write(err, s.as_bytes());
    let _ = write(err, b"\n");
}

/// Print an empty line to stdout.
#[inline]
pub fn println_empty() {
    // SAFETY: stdout() is safe to call - it returns the process's stdout fd
    let _ = write(unsafe { stdout() }, b"\n");
}

/// Print an empty line to stderr.
#[inline]
pub fn eprintln_empty() {
    // SAFETY: stderr() is safe to call - it returns the process's stderr fd
    let _ = write(unsafe { stderr() }, b"\n");
}

/// Print a single character to stdout.
#[inline]
pub fn print_char(c: char) {
    let mut buf = [0u8; 4];
    let s = c.encode_utf8(&mut buf);
    // SAFETY: stdout() is safe to call - it returns the process's stdout fd
    let _ = write(unsafe { stdout() }, s.as_bytes());
}

/// Print a u64 to stdout using itoa.
#[inline]
pub fn print_u64(n: u64) {
    let mut buf = itoa::Buffer::new();
    print(buf.format(n));
}

/// Print a u64 to stdout with newline.
#[inline]
pub fn println_u64(n: u64) {
    let mut buf = itoa::Buffer::new();
    println(buf.format(n));
}

/// Text output writer for KEY=VALUE format.
/// Handles spacing between fields automatically.
pub struct TextWriter {
    first: bool,
}

impl TextWriter {
    /// Create a new text writer.
    pub fn new() -> Self {
        Self { first: true }
    }

    /// Print field separator (space, except for first field).
    fn sep(&mut self) {
        if self.first {
            self.first = false;
        } else {
            print(" ");
        }
    }

    /// Print field name in UPPERCASE (buffered to single syscall).
    fn key(&self, name: &str) {
        // Buffer uppercase conversion - field names are short (max ~20 chars)
        let mut buf = [0u8; 32];
        let len = name.len().min(32);
        for (i, c) in name.bytes().take(len).enumerate() {
            buf[i] = if c >= b'a' && c <= b'z' { c - 32 } else { c };
        }
        // SAFETY: We're converting ASCII lowercase to uppercase, result is valid UTF-8
        let s = unsafe { core::str::from_utf8_unchecked(&buf[..len]) };
        print(s);
    }

    /// Print KEY=value (u64).
    pub fn field_u64(&mut self, name: &str, value: u64) {
        self.sep();
        self.key(name);
        print("=");
        print_u64(value);
    }

    /// Print KEY=value (i64).
    pub fn field_i64(&mut self, name: &str, value: i64) {
        self.sep();
        self.key(name);
        print("=");
        let mut buf = itoa::Buffer::new();
        print(buf.format(value));
    }

    /// Print KEY=value (string, no quotes).
    pub fn field_str(&mut self, name: &str, value: &str) {
        self.sep();
        self.key(name);
        print("=");
        print(value);
    }

    /// Print KEY="value" (string, with quotes).
    pub fn field_quoted(&mut self, name: &str, value: &str) {
        self.sep();
        self.key(name);
        print("=\"");
        print(value);
        print("\"");
    }

    /// Print KEY=value if Some (u64).
    pub fn field_u64_opt(&mut self, name: &str, value: Option<u64>) {
        if let Some(v) = value {
            self.field_u64(name, v);
        }
    }

    /// Print KEY=value if Some (string, no quotes).
    pub fn field_str_opt(&mut self, name: &str, value: Option<&str>) {
        if let Some(v) = value {
            self.field_str(name, v);
        }
    }

    /// Print KEY="value" if Some (string, with quotes).
    pub fn field_quoted_opt(&mut self, name: &str, value: Option<&str>) {
        if let Some(v) = value {
            self.field_quoted(name, v);
        }
    }

    /// Print KEY=value for MHz (fixed point x100).
    pub fn field_mhz(&mut self, name: &str, mhz_x100: u32) {
        self.sep();
        self.key(name);
        print("=");
        let whole = mhz_x100 / 100;
        let frac = mhz_x100 % 100;
        print_u64(whole as u64);
        print(".");
        if frac < 10 {
            print("0");
        }
        print_u64(frac as u64);
    }

    /// Finish the line with a newline.
    pub fn finish(self) {
        println_empty();
    }
}
