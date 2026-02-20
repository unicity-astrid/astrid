//! UTF-8-safe string truncation utilities.

/// Truncate a string at the nearest char boundary at or before `max_bytes`.
///
/// If `s` is already within `max_bytes`, returns a clone. Otherwise, walks
/// backward from `max_bytes` to find the nearest valid `char` boundary and
/// returns the prefix up to that point.
///
/// # Examples
///
/// ```
/// use astrid_tools::truncate_at_char_boundary;
///
/// // ASCII — truncates exactly at max_bytes
/// assert_eq!(truncate_at_char_boundary("hello world", 5), "hello");
///
/// // Multi-byte emoji (🦀 = 4 bytes) — walks back to avoid splitting
/// let s = format!("{}🦀", "x".repeat(198));
/// assert_eq!(truncate_at_char_boundary(&s, 200), "x".repeat(198));
/// ```
#[must_use]
pub fn truncate_at_char_boundary(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    s[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Basic behavior ----

    #[test]
    fn short_string_returned_unchanged() {
        let s = "hello";
        assert_eq!(truncate_at_char_boundary(s, 200), "hello");
    }

    #[test]
    fn empty_string_returned_unchanged() {
        assert_eq!(truncate_at_char_boundary("", 100), "");
    }

    #[test]
    fn exact_length_returned_unchanged() {
        let s = "x".repeat(200);
        assert_eq!(truncate_at_char_boundary(&s, 200), s);
    }

    #[test]
    fn ascii_truncates_at_exact_boundary() {
        let s = "x".repeat(300);
        let result = truncate_at_char_boundary(&s, 200);
        assert_eq!(result.len(), 200);
        assert_eq!(result, "x".repeat(200));
    }

    // ---- Multi-byte characters at the boundary ----

    #[test]
    fn four_byte_emoji_at_boundary() {
        // 🦀 = 4 bytes. Place it at bytes 198..201.
        let mut s = "x".repeat(198);
        s.push('🦀');
        assert_eq!(s.len(), 202);

        let result = truncate_at_char_boundary(&s, 200);
        // Should walk back to 198, not split the emoji.
        assert_eq!(result, "x".repeat(198));
    }

    #[test]
    fn three_byte_char_at_boundary() {
        // '€' (U+20AC) = 3 bytes. Place at bytes 199..201.
        let mut s = "x".repeat(199);
        s.push('€');
        assert_eq!(s.len(), 202);

        let result = truncate_at_char_boundary(&s, 200);
        assert_eq!(result, "x".repeat(199));
    }

    #[test]
    fn two_byte_char_at_boundary() {
        // 'ñ' (U+00F1) = 2 bytes. Place at bytes 199..200.
        let mut s = "x".repeat(199);
        s.push('ñ');
        assert_eq!(s.len(), 201);

        let result = truncate_at_char_boundary(&s, 200);
        // Byte 200 is inside the 2-byte ñ, so walks back to 199.
        assert_eq!(result, "x".repeat(199));
    }

    #[test]
    fn boundary_falls_exactly_on_char_start() {
        // 🦀 = 4 bytes. Place it at 196..199, so byte 200 is the 'y' after it.
        let mut s = "x".repeat(196);
        s.push('🦀'); // bytes 196..199
        s.push('y'); // byte 200
        s.push_str(&"z".repeat(50));
        assert!(s.len() > 200);

        let result = truncate_at_char_boundary(&s, 200);
        // Byte 200 is a valid boundary, so s[..200] = 196 x's + 🦀 (200 bytes).
        // 'y' at byte 200 is excluded because [..200] is exclusive of the end.
        assert_eq!(result.len(), 200);
        let mut expected = "x".repeat(196);
        expected.push('🦀');
        assert_eq!(result, expected);
    }

    // ---- All multi-byte content ----

    #[test]
    fn all_multibyte_chars() {
        // 50 x 🦀 = 200 bytes exactly. Adding one more exceeds.
        let s = "🦀".repeat(51); // 204 bytes
        let result = truncate_at_char_boundary(&s, 200);
        // Should truncate to 50 crabs = 200 bytes.
        assert_eq!(result, "🦀".repeat(50));
        assert_eq!(result.len(), 200);
    }

    // ---- Edge: max_bytes = 0 ----

    #[test]
    fn zero_max_bytes_returns_empty() {
        assert_eq!(truncate_at_char_boundary("hello", 0), "");
    }

    // ---- Result is always valid UTF-8 ----

    #[test]
    fn result_is_valid_utf8() {
        // Mix of different multi-byte widths
        let s = "añ€🦀".repeat(100); // 10 bytes per repeat × 100 = 1000 bytes
        for boundary in [1, 2, 3, 4, 5, 50, 100, 500, 999] {
            let result = truncate_at_char_boundary(&s, boundary);
            // If this line compiles and doesn't panic, it's valid UTF-8.
            assert!(result.len() <= boundary);
            assert!(result.is_char_boundary(result.len()));
        }
    }
}
