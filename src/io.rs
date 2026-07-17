//! I/O helpers for reading sysfs and procfs.
//!
//! These functions are designed to be forgiving - they return Option<T> rather
//! than Result<T, E> because in the world of /sys and /proc, files come and go,
//! permissions vary, and we'd rather skip a field than crash.
//!
//! Philosophy: "If you can't read it, shrug and move on."
//!
//! Uses rustix for direct syscalls to minimize binary size.
//! no_std compatible - uses stack-based types instead of String/Vec.

use core::mem::MaybeUninit;
use core::str::FromStr;

use rustix::fs::{openat, Mode, OFlags, RawDir, CWD};
use rustix::io::read;

use crate::stack::StackString;

// ============================================================================
// Directory iteration (stack-based, no allocation)
// ============================================================================

/// Iterate over directory entries, calling a callback for each name.
/// Skips "." and ".." entries automatically.
/// This is a simple callback-based approach that avoids complex iterator state.
pub fn for_each_dir_entry<F>(path: &str, mut callback: F)
where
    F: FnMut(&str),
{
    let Ok(fd) = openat(CWD, path, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty()) else {
        return;
    };

    // RawDir::next refills its buffer via getdents until the directory is
    // exhausted, so one pass is enough.
    let mut buf: [MaybeUninit<u8>; 2048] = [MaybeUninit::uninit(); 2048];
    let mut raw_dir = RawDir::new(&fd, &mut buf);
    let mut count = 0usize;
    while let Some(entry_result) = raw_dir.next() {
        // A getdents error means the rest of the listing is unreadable
        let Ok(entry) = entry_result else { break };
        let name_bytes = entry.file_name().to_bytes();
        // Skip . and ..
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }
        if let Ok(name_str) = core::str::from_utf8(name_bytes) {
            count += 1;
            callback(name_str);
        }
    }
    crate::debug::scan(path, count);
}

/// Read a symlink target into a StackString.
/// Returns the link target as stored (e.g. "../../../drivers/virtio_net"),
/// truncated if it exceeds the internal 256-byte buffer.
pub fn read_symlink<const N: usize>(path: &str) -> Option<StackString<N>> {
    let mut buf = [MaybeUninit::<u8>::uninit(); 256];
    let (target, _) = rustix::fs::readlinkat_raw(CWD, path, &mut buf).ok()?;
    let link_path = core::str::from_utf8(target).ok()?;
    Some(StackString::from_str(link_path))
}

/// Read a symlink and extract just the final component (filename).
/// Used for reading driver symlinks like /sys/bus/pci/devices/XXX/driver -> ../../../drivers/NAME
pub fn read_symlink_name<const N: usize>(path: &str) -> Option<StackString<N>> {
    let link: StackString<256> = read_symlink(path)?;
    let name = link.as_str().rsplit('/').next()?;
    if name.is_empty() {
        None
    } else {
        Some(StackString::from_str(name))
    }
}

/// Hex digits lookup table.
const HEX_DIGITS: [u8; 16] = *b"0123456789abcdef";

/// Trait for converting bytes to hex characters.
pub trait HexNibble {
    /// High nibble as hex character (e.g., 0xAB -> 'a').
    fn hex_hi(self) -> char;
    /// Low nibble as hex character (e.g., 0xAB -> 'b').
    fn hex_lo(self) -> char;
}

impl HexNibble for u8 {
    #[inline]
    fn hex_hi(self) -> char {
        HEX_DIGITS[(self >> 4) as usize] as char
    }
    #[inline]
    fn hex_lo(self) -> char {
        HEX_DIGITS[(self & 0xf) as usize] as char
    }
}

// ============================================================================
// Core file reading functions (stack-based, no allocation)
// ============================================================================

/// Read up to buf.len() raw bytes from a file, looping until the buffer is
/// full or EOF: a single read() is not enough for procfs files like
/// /proc/cpuinfo, which come in chunks and easily exceed 4 KiB on
/// multi-core machines. No UTF-8 or content checks.
/// Returns the number of bytes read.
pub fn read_file_raw(path: &str, buf: &mut [u8]) -> Option<usize> {
    let fd = match openat(CWD, path, OFlags::RDONLY, Mode::empty()) {
        Ok(fd) => fd,
        Err(_e) => {
            crate::dbg_fail!(path, _e);
            return None;
        }
    };

    let mut len = 0;
    while len < buf.len() {
        match read(&fd, &mut buf[len..]) {
            Ok(0) => break,
            Ok(n) => len += n,
            Err(_e) => {
                crate::dbg_fail!(path, _e);
                return None;
            }
        }
    }
    Some(len)
}

/// Read a file into a stack buffer and return trimmed content.
/// Reads up to N bytes, anything past that is dropped (pick N generously).
/// Returns None if the file can't be read, is empty, or isn't valid UTF-8.
pub fn read_file_stack<const N: usize>(path: &str) -> Option<StackString<N>> {
    let mut buf = [0u8; N];
    let len = read_file_raw(path, &mut buf)?;

    // Convert to string and trim. If we filled the buffer exactly, the cut
    // may have split a multi-byte character - keep the valid prefix then.
    let s = match core::str::from_utf8(&buf[..len]) {
        Ok(s) => s.trim(),
        Err(e) if len == N && len - e.valid_up_to() < 4 => {
            core::str::from_utf8(&buf[..e.valid_up_to()]).ok()?.trim()
        }
        Err(_) => return None,
    };

    if s.is_empty() {
        None
    } else {
        Some(StackString::from_str(s))
    }
}

/// Read a file and parse it as type T (stack-based, no allocation).
pub fn read_file_parse<T: FromStr>(path: &str) -> Option<T> {
    let s: StackString<64> = read_file_stack(path)?;
    s.as_str().parse().ok()
}

/// Trait for types that can be parsed from a string with a radix.
pub trait FromStrRadix: Sized {
    fn from_str_radix(s: &str, radix: u32) -> Option<Self>;
}

// Implement for the integer types we care about
macro_rules! impl_from_str_radix {
    ($($t:ty),*) => {
        $(
            impl FromStrRadix for $t {
                fn from_str_radix(s: &str, radix: u32) -> Option<Self> {
                    <$t>::from_str_radix(s, radix).ok()
                }
            }
        )*
    };
}

impl_from_str_radix!(u8, u16, u32, u64, usize, i32, i64);

/// Parse a hex string, with or without "0x" prefix.
pub fn parse_hex<T: FromStrRadix>(s: &str) -> Option<T> {
    let s = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    T::from_str_radix(s, 16)
}

/// Read a file containing a hexadecimal value (stack-based).
pub fn read_file_hex<T: FromStrRadix>(path: &str) -> Option<T> {
    let s: StackString<64> = read_file_stack(path)?;
    parse_hex(s.as_str())
}

/// Check if a path exists.
pub fn path_exists(path: &str) -> bool {
    rustix::fs::access(path, rustix::fs::Access::EXISTS).is_ok()
}

/// Check if a path is a directory (not following symlinks).
pub fn is_dir(path: &str) -> bool {
    match rustix::fs::lstat(path) {
        Ok(stat) => rustix::fs::FileType::from_raw_mode(stat.st_mode) == rustix::fs::FileType::Directory,
        Err(_) => false,
    }
}

/// Check if a path is a regular file (not following symlinks).
pub fn is_file(path: &str) -> bool {
    match rustix::fs::lstat(path) {
        Ok(stat) => rustix::fs::FileType::from_raw_mode(stat.st_mode) == rustix::fs::FileType::RegularFile,
        Err(_) => false,
    }
}

/// Check if a path is a symlink.
pub fn is_symlink(path: &str) -> bool {
    match rustix::fs::lstat(path) {
        Ok(stat) => rustix::fs::FileType::from_raw_mode(stat.st_mode) == rustix::fs::FileType::Symlink,
        Err(_) => false,
    }
}

/// Get the size of a file in bytes (using lstat - doesn't follow symlinks).
pub fn file_size(path: &str) -> Option<u64> {
    rustix::fs::lstat(path).ok().map(|stat| stat.st_size as u64)
}

// ============================================================================
// Path manipulation (stack-based)
// ============================================================================

/// Join a base path with a filename into a StackString.
#[inline]
pub fn join_path<const N: usize>(base: &str, name: &str) -> StackString<N> {
    let mut path = StackString::new();
    path.push_str(base);
    if !base.ends_with('/') {
        path.push('/');
    }
    path.push_str(name);
    path
}

// ============================================================================
// Formatting functions (stack-based, no allocation)
// ============================================================================

/// Format a u16 as a hex string with "0x" prefix into a StackString.
pub fn format_hex_u16(val: u16) -> StackString<16> {
    let mut s = StackString::new();
    s.push_str("0x");
    let hi = (val >> 8) as u8;
    let lo = val as u8;
    s.push(hi.hex_hi());
    s.push(hi.hex_lo());
    s.push(lo.hex_hi());
    s.push(lo.hex_lo());
    s
}

/// Format a u8 as a hex string with "0x" prefix into a StackString.
pub fn format_hex_u8(val: u8) -> StackString<16> {
    let mut s = StackString::new();
    s.push_str("0x");
    s.push(val.hex_hi());
    s.push(val.hex_lo());
    s
}

/// Format a u32 as a 6-digit hex string with "0x" prefix (for PCI class codes).
pub fn format_hex_class(val: u32) -> StackString<16> {
    let mut s = StackString::new();
    s.push_str("0x");
    let b2 = ((val >> 16) & 0xff) as u8;
    let b1 = ((val >> 8) & 0xff) as u8;
    let b0 = (val & 0xff) as u8;
    s.push(b2.hex_hi());
    s.push(b2.hex_lo());
    s.push(b1.hex_hi());
    s.push(b1.hex_lo());
    s.push(b0.hex_hi());
    s.push(b0.hex_lo());
    s
}

/// Extension trait for converting KB values to bytes.
pub trait KbToBytes {
    fn kb(self) -> u64;
}

impl KbToBytes for u64 {
    #[inline]
    fn kb(self) -> u64 {
        self * 1024
    }
}

/// Format bytes as human-readable size (e.g., "16G", "512M", "4K").
pub fn format_human_size(bytes: u64) -> StackString<16> {
    const KI: u64 = 1024;
    const MI: u64 = 1024 * 1024;
    const GI: u64 = 1024 * 1024 * 1024;
    const TI: u64 = 1024 * 1024 * 1024 * 1024;

    let mut s = StackString::new();
    let mut buf = itoa::Buffer::new();

    if bytes >= TI {
        s.push_str(buf.format(bytes / TI));
        s.push('T');
    } else if bytes >= GI {
        s.push_str(buf.format(bytes / GI));
        s.push('G');
    } else if bytes >= MI {
        s.push_str(buf.format(bytes / MI));
        s.push('M');
    } else if bytes >= KI {
        s.push_str(buf.format(bytes / KI));
        s.push('K');
    } else {
        s.push_str(buf.format(bytes));
    }
    s
}

/// Format a sector count as human-readable size (e.g., "500G", "1T").
pub fn format_sectors_human(sectors: u64, sector_size: u32) -> StackString<16> {
    let bytes = sectors * sector_size as u64;
    format_human_size(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> std::string::String {
        let mut p = std::env::temp_dir();
        p.push(std::format!("kv-io-test-{}-{}", std::process::id(), name));
        p.to_str().unwrap().to_string()
    }

    #[test]
    fn read_file_stack_reads_past_4096() {
        // /proc/cpuinfo on many-core machines and /proc/self/mounts on
        // container hosts are well over 4 KiB, and truncating them corrupts output.
        let path = tmp_path("big");
        let content = "x".repeat(6000);
        std::fs::write(&path, &content).unwrap();
        let s: Option<StackString<8192>> = read_file_stack(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(s.expect("readable").len(), 6000);
    }

    #[test]
    fn read_file_stack_still_truncates_at_capacity() {
        let path = tmp_path("cap");
        let content = "y".repeat(300);
        std::fs::write(&path, &content).unwrap();
        let s: Option<StackString<64>> = read_file_stack(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(s.expect("readable").len(), 64);
    }

    #[test]
    fn read_file_raw_reads_binary_and_empty() {
        let path = tmp_path("raw");
        std::fs::write(&path, [0x00u8, 0x01, 0x80, 0xff]).unwrap();
        let mut buf = [0u8; 16];
        let n = read_file_raw(&path, &mut buf);
        assert_eq!(n, Some(4));
        assert_eq!(&buf[..4], &[0x00, 0x01, 0x80, 0xff]);

        std::fs::write(&path, b"").unwrap();
        let n = read_file_raw(&path, &mut buf);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(n, Some(0));
    }

    #[test]
    fn read_symlink_name_returns_target_basename() {
        // Mirrors sysfs driver links: driver -> ../../../drivers/virtio_net
        let link = tmp_path("driver-link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink("../../../drivers/virtio_net", &link).unwrap();
        let name: Option<StackString<64>> = read_symlink_name(&link);
        std::fs::remove_file(&link).unwrap();
        assert_eq!(name.as_deref(), Some("virtio_net"));
    }

    #[test]
    fn parse_hex_with_and_without_prefix() {
        assert_eq!(parse_hex::<u16>("0x1af4"), Some(0x1af4));
        assert_eq!(parse_hex::<u16>("1af4"), Some(0x1af4));
        assert_eq!(parse_hex::<u16>("zz"), None);
    }

    #[test]
    fn format_human_size_boundaries() {
        assert_eq!(format_human_size(512).as_str(), "512");
        assert_eq!(format_human_size(2048).as_str(), "2K");
        assert_eq!(format_human_size(3 * 1024 * 1024 * 1024).as_str(), "3G");
    }
}
