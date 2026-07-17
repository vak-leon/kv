//! Stack-allocated string and buffer types for no-alloc operation.
//!
//! These types provide fixed-capacity alternatives to String and Vec
//! that don't require a heap allocator. They're designed for the
//! constrained environment of sysfs/procfs reading where we know
//! reasonable upper bounds on data sizes.

/// A stack-allocated string with fixed capacity.
/// N is the capacity in bytes.
#[derive(Clone)]
pub struct StackString<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> StackString<N> {
    /// Create a new empty StackString.
    #[inline]
    pub const fn new() -> Self {
        Self {
            buf: [0u8; N],
            len: 0,
        }
    }

    /// Create from a string slice, truncating if necessary.
    #[inline]
    pub fn from_str(s: &str) -> Self {
        let mut this = Self::new();
        this.push_str(s);
        this
    }

    /// Get the string as a slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        // SAFETY: We only ever write valid UTF-8
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) }
    }

    /// Get the length in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get remaining capacity.
    #[inline]
    pub fn remaining(&self) -> usize {
        N - self.len
    }

    /// Push a character, returning false if full.
    #[inline]
    pub fn push(&mut self, c: char) -> bool {
        let mut buf = [0u8; 4];
        let encoded = c.encode_utf8(&mut buf);
        self.push_str(encoded)
    }

    /// Push a string slice, truncating if needed. Returns true if all of it fit.
    #[inline]
    pub fn push_str(&mut self, s: &str) -> bool {
        let bytes = s.as_bytes();
        let mut to_copy = bytes.len().min(self.remaining());
        // Never split a multi-byte character: as_str() relies on the buffer
        // always holding valid UTF-8.
        while to_copy < bytes.len() && !s.is_char_boundary(to_copy) {
            to_copy -= 1;
        }
        if to_copy > 0 {
            self.buf[self.len..self.len + to_copy].copy_from_slice(&bytes[..to_copy]);
            self.len += to_copy;
        }
        to_copy == bytes.len()
    }

    /// Clear the string.
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Trim whitespace from both ends (returns a new StackString).
    pub fn trim(&self) -> StackString<N> {
        StackString::from_str(self.as_str().trim())
    }
}

impl<const N: usize> Default for StackString<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> core::ops::Deref for StackString<N> {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl<const N: usize> AsRef<str> for StackString<N> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// A stack-allocated buffer for reading files.
pub struct StackBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> StackBuf<N> {
    /// Create a new empty buffer.
    #[inline]
    pub const fn new() -> Self {
        Self {
            buf: [0u8; N],
            len: 0,
        }
    }

    /// Get the buffer as a mutable slice for reading into.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buf
    }

    /// Set the length after reading.
    #[inline]
    pub fn set_len(&mut self, len: usize) {
        self.len = len.min(N);
    }

    /// Get the filled portion as a byte slice.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Try to interpret as UTF-8 string.
    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.buf[..self.len]).ok()
    }

    /// Get as trimmed string.
    pub fn as_str_trimmed(&self) -> Option<&str> {
        self.as_str().map(|s| s.trim())
    }
}

impl<const N: usize> Default for StackBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Push an integer to a StackString using itoa.
pub fn push_u64<const N: usize>(s: &mut StackString<N>, val: u64) {
    let mut buf = itoa::Buffer::new();
    s.push_str(buf.format(val));
}

/// Push a signed fixed-point value; `scaled` is the value times 10^decimals.
/// e.g. push_fixed_point(s, -80, 2) writes "-0.80".
/// The sign is emitted explicitly so small negatives don't collapse to "-0"
/// losing the minus (integer division rounds -0.8 to 0).
pub fn push_fixed_point<const N: usize>(s: &mut StackString<N>, scaled: i64, decimals: u32) {
    if scaled < 0 {
        s.push('-');
    }
    let mag = scaled.unsigned_abs();
    let div = 10u64.pow(decimals);
    let whole = mag / div;
    let frac = mag % div;
    let mut buf = itoa::Buffer::new();
    s.push_str(buf.format(whole));
    if decimals > 0 {
        s.push('.');
        let mut pad = div / 10;
        while pad > 1 && frac < pad {
            s.push('0');
            pad /= 10;
        }
        s.push_str(buf.format(frac));
    }
}

/// Push an integer to a StackString using itoa.
pub fn push_i64<const N: usize>(s: &mut StackString<N>, val: i64) {
    let mut buf = itoa::Buffer::new();
    s.push_str(buf.format(val));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stack_string_basic() {
        let mut s: StackString<32> = StackString::new();
        assert!(s.is_empty());
        s.push_str("hello");
        assert_eq!(s.as_str(), "hello");
        s.push(' ');
        s.push_str("world");
        assert_eq!(s.as_str(), "hello world");
    }

    #[test]
    fn test_stack_string_truncate() {
        let mut s: StackString<5> = StackString::new();
        s.push_str("hello world"); // Should truncate
        assert_eq!(s.as_str(), "hello");
        assert_eq!(s.len(), 5);
    }

    #[test]
    fn test_stack_string_trim() {
        let s: StackString<32> = StackString::from_str("  hello  ");
        let trimmed = s.trim();
        assert_eq!(trimmed.as_str(), "hello");
    }

    #[test]
    fn push_str_truncates_at_char_boundary() {
        // U+00A3 is 2 bytes but only 1 byte of capacity remains. A raw
        // byte-wise truncation would leave invalid UTF-8 behind as_str()'s
        // from_utf8_unchecked - that's UB, not just a wrong string.
        let mut s: StackString<4> = StackString::new();
        let fit = s.push_str("abc\u{a3}");
        assert!(!fit);
        assert_eq!(s.len(), 3);
        assert_eq!(s.as_str(), "abc");
    }

    #[test]
    fn push_multibyte_char_into_full_string_is_rejected() {
        let mut s: StackString<1> = StackString::new();
        assert!(!s.push('\u{a3}'));
        assert_eq!(s.len(), 0);
        assert_eq!(s.as_str(), "");
    }

    #[test]
    fn push_str_multibyte_fits_exactly() {
        let mut s: StackString<5> = StackString::new();
        assert!(s.push_str("abc\u{a3}"));
        assert_eq!(s.as_str(), "abc\u{a3}");
    }

    fn fixed(scaled: i64, decimals: u32) -> std::string::String {
        let mut s: StackString<32> = StackString::new();
        push_fixed_point(&mut s, scaled, decimals);
        s.as_str().to_string()
    }

    #[test]
    fn fixed_point_positive() {
        assert_eq!(fixed(1250, 2), "12.50");
        assert_eq!(fixed(125, 1), "12.5");
        assert_eq!(fixed(800, 0), "800");
    }

    #[test]
    fn fixed_point_pads_fraction() {
        assert_eq!(fixed(1205, 2), "12.05");
        assert_eq!(fixed(1200, 2), "12.00");
        assert_eq!(fixed(12080, 3), "12.080");
    }

    #[test]
    fn fixed_point_keeps_sign_below_one() {
        assert_eq!(fixed(-80, 2), "-0.80");
        assert_eq!(fixed(-5, 1), "-0.5");
        assert_eq!(fixed(-800, 3), "-0.800");
    }
}
