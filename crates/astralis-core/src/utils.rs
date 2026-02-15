//! Utility functions for the Astralis core library.

/// Truncate a string to at most `max_bytes`, ensuring the cut falls on a
/// UTF-8 character boundary. Returns the original string if already short enough.
#[must_use]
pub fn truncate_to_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        // Safety: end > 0 is checked by the loop condition
        #[allow(clippy::arithmetic_side_effects)]
        {
            end -= 1;
        }
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_no_truncation() {
        assert_eq!(truncate_to_boundary("hello", 10), "hello");
    }

    #[test]
    fn ascii_truncation() {
        assert_eq!(truncate_to_boundary("hello world", 5), "hello");
    }

    #[test]
    fn emoji_boundary() {
        // Each emoji is 4 bytes
        let s = "😀😁😂";
        assert_eq!(truncate_to_boundary(s, 4), "😀");
        assert_eq!(truncate_to_boundary(s, 5), "😀");
        assert_eq!(truncate_to_boundary(s, 7), "😀");
        assert_eq!(truncate_to_boundary(s, 8), "😀😁");
    }

    #[test]
    fn multibyte_boundary() {
        // 'é' is 2 bytes in UTF-8
        let s = "café";
        assert_eq!(truncate_to_boundary(s, 3), "caf");
        assert_eq!(truncate_to_boundary(s, 4), "caf");
        assert_eq!(truncate_to_boundary(s, 5), "café");
    }

    #[test]
    fn zero_max_bytes() {
        assert_eq!(truncate_to_boundary("hello", 0), "");
    }

    #[test]
    fn empty_string() {
        assert_eq!(truncate_to_boundary("", 10), "");
    }
}
