//! Mount point information from /proc/self/mounts.
//!
//! Shows what's mounted where, which filesystem type, and mount options.
//! Useful when you need to know if that NFS share actually mounted or
//! why your rootfs is mysteriously read-only.

use crate::cli::GlobalOptions;
use crate::fields::mounts as f;
use crate::filter::matches_any;
use crate::io;
use crate::json::{begin_kv_output_streaming, StreamingJsonWriter};
use crate::print::{self, TextWriter};
use crate::stack::StackString;

const MOUNTS_PATH: &str = "/proc/self/mounts";

/// A single mount point entry.
pub struct MountEntry {
    /// Source device or pseudo-filesystem (e.g., "/dev/sda1" or "tmpfs")
    pub source: StackString<256>,
    /// Target mount point (e.g., "/" or "/home")
    pub target: StackString<256>,
    /// Filesystem type (e.g., "ext4", "tmpfs", "nfs")
    pub fstype: StackString<64>,
    /// Mount options as a single string
    pub options: StackString<512>,
    /// Dump frequency (from fstab, usually 0)
    pub dump_freq: u32,
    /// fsck pass number (usually 0)
    pub pass_num: u32,
}

impl MountEntry {
    /// Parse a line from /proc/mounts.
    ///
    /// Format: device mountpoint fstype options dump pass
    /// Example: /dev/sda1 / ext4 rw,relatime 0 0
    ///
    /// Note: Spaces in paths are escaped as \040 (octal), and we decode them.
    pub fn parse(line: &str) -> Option<Self> {
        let mut parts = line.split_whitespace();

        let source_raw = parts.next()?;
        let target_raw = parts.next()?;
        let fstype = parts.next()?;
        let options = parts.next()?;

        // dump and pass are optional
        let dump_freq: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let pass_num: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);

        Some(MountEntry {
            source: decode_mount_escapes(source_raw),
            target: decode_mount_escapes(target_raw),
            fstype: StackString::from_str(fstype),
            options: StackString::from_str(options),
            dump_freq,
            pass_num,
        })
    }

    /// Check if this mount matches the filter pattern.
    fn matches_filter(&self, pattern: &str, case_insensitive: bool) -> bool {
        let fields = [self.source.as_str(), self.target.as_str(), self.fstype.as_str()];
        matches_any(&fields, pattern, case_insensitive)
    }

    /// Output as text (single line, KEY=VALUE format).
    fn print_text(&self) {
        let mut w = TextWriter::new();
        w.field_quoted(f::SOURCE, self.source.as_str());
        w.field_quoted(f::TARGET, self.target.as_str());
        w.field_str(f::FSTYPE, self.fstype.as_str());
        w.field_quoted(f::OPTIONS, self.options.as_str());
        w.finish();
    }

    /// Output as JSON object fields.
    fn write_json(&self, w: &mut StreamingJsonWriter, verbose: bool) {
        w.array_object_begin();
        w.field_str(f::SOURCE, self.source.as_str());
        w.field_str(f::TARGET, self.target.as_str());
        w.field_str(f::FSTYPE, self.fstype.as_str());
        w.field_str(f::OPTIONS, self.options.as_str());
        if verbose {
            w.field_u64(f::DUMP_FREQ, self.dump_freq as u64);
            w.field_u64(f::PASS_NUM, self.pass_num as u64);
        }
        w.array_object_end();
    }
}

/// Decode mount escape sequences.
///
/// The kernel escapes special characters in mount paths using octal:
/// - Space: \040
/// - Tab: \011
/// - Newline: \012
/// - Backslash: \134
///
/// This is important for paths like "/mnt/My Documents" which would
/// otherwise confuse the space-separated format.
fn decode_mount_escapes(s: &str) -> StackString<256> {
    let mut result: StackString<256> = StackString::new();
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            // Check if next 3 chars are octal digits
            let d1 = bytes[i + 1];
            let d2 = bytes[i + 2];
            let d3 = bytes[i + 3];
            if is_octal_digit(d1) && is_octal_digit(d2) && is_octal_digit(d3) {
                // Widened math: \400-\777 would overflow u8 (not a byte value;
                // the kernel never emits those, but don't panic on them either)
                let val = ((d1 - b'0') as u32) * 64 + ((d2 - b'0') as u32) * 8 + (d3 - b'0') as u32;
                if val <= 255 {
                    result.push(val as u8 as char);
                    i += 4;
                    continue;
                }
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }

    result
}

#[inline]
fn is_octal_digit(b: u8) -> bool {
    b >= b'0' && b <= b'7'
}

#[cfg(test)]
mod escape_tests {
    use super::*;

    #[test]
    fn decodes_kernel_octal_escapes() {
        assert_eq!(
            decode_mount_escapes("/mnt/My\\040Documents").as_str(),
            "/mnt/My Documents"
        );
        assert_eq!(decode_mount_escapes("a\\134b").as_str(), "a\\b");
        assert_eq!(decode_mount_escapes("tab\\011sep").as_str(), "tab\tsep");
    }

    #[test]
    fn passes_through_non_escapes() {
        assert_eq!(decode_mount_escapes("/plain/path").as_str(), "/plain/path");
        assert_eq!(decode_mount_escapes("trailing\\04").as_str(), "trailing\\04");
    }

    #[test]
    fn decodes_high_octal_bytes() {
        assert_eq!(decode_mount_escapes("\\377").as_str(), "\u{ff}");
    }

    #[test]
    fn out_of_range_octal_is_left_literal() {
        // \777 = 511, not a byte. Kernel never emits it, but arithmetic on
        // u8 must not overflow (panics in dev builds).
        assert_eq!(decode_mount_escapes("\\777").as_str(), "\\777");
    }
}

/// Entry point for `kv mounts` subcommand.
pub fn run(opts: &GlobalOptions) -> i32 {
    // Read the entire mounts file
    let contents: StackString<32768> = match io::read_file_stack(MOUNTS_PATH) {
        Some(c) => c,
        None => {
            if opts.json {
                let mut w = begin_kv_output_streaming(opts.pretty, "mounts");
                w.field_array("data");
                w.end_field_array();
                w.end_object();
                w.finish();
            } else {
                print::println("mounts: no mounts found (is /proc mounted?)");
            }
            return 0;
        }
    };

    let filter = opts.filter.as_ref().map(|s| s.as_str());
    let case_insensitive = opts.filter_case_insensitive;

    if opts.json {
        let mut w = begin_kv_output_streaming(opts.pretty, "mounts");
        w.field_array("data");

        for line in contents.as_str().lines() {
            if let Some(mount) = MountEntry::parse(line) {
                // Apply filter if present
                if let Some(pattern) = filter {
                    if !mount.matches_filter(pattern, case_insensitive) {
                        continue;
                    }
                }
                mount.write_json(&mut w, opts.verbose);
            }
        }

        w.end_field_array();
        w.end_object();
        w.finish();
    } else {
        let mut count = 0;
        for line in contents.as_str().lines() {
            if let Some(mount) = MountEntry::parse(line) {
                // Apply filter if present
                if let Some(pattern) = filter {
                    if !mount.matches_filter(pattern, case_insensitive) {
                        continue;
                    }
                }
                mount.print_text();
                count += 1;
            }
        }

        if count == 0 {
            if filter.is_some() {
                print::println("mounts: no matching mounts");
            } else {
                print::println("mounts: no mounts found");
            }
        }
    }

    0
}

/// Write mounts to JSON writer (for snapshot).
#[cfg(feature = "snapshot")]
pub fn write_snapshot(w: &mut StreamingJsonWriter, verbose: bool) {
    let contents: StackString<32768> = match io::read_file_stack(MOUNTS_PATH) {
        Some(c) => c,
        None => return,
    };

    w.key("mounts");
    w.begin_array();
    for line in contents.as_str().lines() {
        if let Some(mount) = MountEntry::parse(line) {
            mount.write_json(w, verbose);
        }
    }
    w.end_array();
}
