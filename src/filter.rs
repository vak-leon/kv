//! Filter matching utilities.
//!
//! Provides centralized case-sensitive/insensitive matching so individual
//! subcommand modules don't need to handle this logic.
//!
//! # For Contributors
//!
//! When adding a new subcommand with filterable items, implement the `Filterable`
//! trait on your struct. You only need to implement `filter_fields()` - the
//! `matches_filter()` method is provided automatically.

/// Check if any of the given fields contain the pattern.
///
/// When `case_insensitive` is true, the pattern is assumed to be already
/// lowercased (done by CLI parser when `-F` is used) and matching folds
/// ASCII case only - plenty for sysfs names, and it keeps Unicode case
/// tables out of the binary.
pub fn matches_any(fields: &[&str], pattern: &str, case_insensitive: bool) -> bool {
    if case_insensitive {
        fields.iter().any(|f| contains_ascii_insensitive(f, pattern))
    } else {
        fields.iter().any(|f| f.contains(pattern))
    }
}

/// Substring search that lowercases the haystack byte-by-byte (ASCII only).
/// Pattern must already be lowercase. No scratch buffer, no length limit.
fn contains_ascii_insensitive(field: &str, pattern: &str) -> bool {
    let f = field.as_bytes();
    let p = pattern.as_bytes();
    if p.is_empty() {
        return true;
    }
    if f.len() < p.len() {
        return false;
    }
    'candidates: for start in 0..=(f.len() - p.len()) {
        for (i, &pc) in p.iter().enumerate() {
            if f[start + i].to_ascii_lowercase() != pc {
                continue 'candidates;
            }
        }
        return true;
    }
    false
}

/// Extract `&str` from `Option<T>` where T implements AsRef<str>.
/// Returns `""` if `None`.
#[inline]
pub fn opt_str<T: AsRef<str>>(opt: &Option<T>) -> &str {
    opt.as_ref().map(|s| s.as_ref()).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_sensitive_match() {
        assert!(matches_any(&["wlp3s0", "eth0"], "wlp", false));
        assert!(!matches_any(&["WLP3S0"], "wlp", false));
    }

    #[test]
    fn case_insensitive_match() {
        // The CLI lowercases the pattern before we get here
        assert!(matches_any(&["WLP3S0"], "wlp", true));
        assert!(matches_any(&["MiXeD"], "mixed", true));
        assert!(!matches_any(&["abc"], "xyz", true));
    }

    #[test]
    fn case_insensitive_matches_long_field_tail() {
        // Fields longer than any internal scratch buffer must still match.
        let mut long = std::string::String::new();
        for _ in 0..300 {
            long.push('A');
        }
        long.push_str("NEEDLE");
        assert!(matches_any(&[long.as_str()], "needle", true));
    }

    #[test]
    fn empty_pattern_matches_everything() {
        assert!(matches_any(&["anything"], "", true));
        assert!(matches_any(&["anything"], "", false));
    }
}
