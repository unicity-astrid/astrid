//! Platform-agnostic text splitting utilities.
//!
//! These functions split text into chunks that fit within a platform's message
//! size limit, preferring natural boundaries (paragraph breaks, newlines) over
//! hard cuts.
//!
//! Platform-specific formatting (e.g., Telegram HTML, Discord markdown) stays
//! in each frontend crate.

/// Default maximum chunk length if none is specified.
const DEFAULT_MAX_LEN: usize = 4000;

/// Split text into chunks that fit within `max_len` bytes.
///
/// Tries to split at paragraph boundaries first (`\n\n`), then newlines
/// (`\n`), then hard-cuts at the byte boundary. All chunks are valid UTF-8.
///
/// If `max_len` is `0`, the [`DEFAULT_MAX_LEN`] (4000) is used.
pub fn chunk_text(text: &str, max_len: usize) -> Vec<String> {
    let max_len = if max_len == 0 {
        DEFAULT_MAX_LEN
    } else {
        max_len
    };

    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Try splitting at a double newline (paragraph boundary).
        let hard_cut = remaining.floor_char_boundary(max_len);
        let split_at = find_split_point(remaining, hard_cut, "\n\n")
            .or_else(|| find_split_point(remaining, hard_cut, "\n"))
            .unwrap_or(hard_cut);

        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(chunk.to_string());
        remaining = rest.trim_start_matches('\n');
    }

    chunks
}

/// Find a split point by searching backwards from `boundary` for `delimiter`.
///
/// Returns the position just after the delimiter, or `None` if the delimiter
/// is not found in `text[..boundary]`.
///
/// `boundary` must be a valid char boundary in `text`.
pub fn find_split_point(text: &str, boundary: usize, delimiter: &str) -> Option<usize> {
    let search_region = &text[..boundary];
    search_region
        .rfind(delimiter)
        .map(|pos| pos.saturating_add(delimiter.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_text_short_message() {
        let chunks = chunk_text("short text", 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "short text");
    }

    #[test]
    fn chunk_text_exact_limit() {
        let text = "x".repeat(100);
        let chunks = chunk_text(&text, 100);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn chunk_text_splits_at_paragraph() {
        let text = "a".repeat(50) + "\n\n" + &"b".repeat(50);
        let chunks = chunk_text(&text, 60);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].starts_with('a'));
        assert!(chunks[1].starts_with('b'));
    }

    #[test]
    fn chunk_text_splits_at_newline_when_no_paragraph() {
        let text = "a".repeat(30) + "\n" + &"b".repeat(30);
        let chunks = chunk_text(&text, 40);
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn chunk_text_hard_split_when_no_breaks() {
        let text = "x".repeat(200);
        let chunks = chunk_text(&text, 100);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 100);
        assert_eq!(chunks[1].len(), 100);
    }

    #[test]
    fn chunk_text_zero_max_uses_default() {
        let text = "short";
        let chunks = chunk_text(text, 0);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn chunk_text_empty_string() {
        let chunks = chunk_text("", 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "");
    }

    #[test]
    fn chunk_text_multibyte_safe() {
        // 3-byte chars — a max_len that falls mid-char must not panic.
        let text = "\u{3042}".repeat(100); // 300 bytes
        let chunks = chunk_text(&text, 50);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            // All chunks must be valid UTF-8 (implicit by being String).
            assert!(!chunk.is_empty());
        }
    }

    #[test]
    fn chunk_text_preserves_all_content() {
        let text = "a".repeat(150) + "\n\n" + &"b".repeat(150);
        let chunks = chunk_text(&text, 200);
        let reassembled: String = chunks.join("");
        assert!(reassembled.contains(&"a".repeat(150)));
        assert!(reassembled.contains(&"b".repeat(150)));
    }

    // --- find_split_point ---

    #[test]
    fn find_split_at_double_newline() {
        let text = "hello\n\nworld and more text here";
        let point = find_split_point(text, 20, "\n\n");
        assert_eq!(point, Some(7)); // After "hello\n\n"
    }

    #[test]
    fn find_split_no_delimiter() {
        let text = "no breaks at all in this text";
        let point = find_split_point(text, 15, "\n\n");
        assert!(point.is_none());
    }

    #[test]
    fn chunk_text_single_char_limit() {
        // Extreme: max_len = 1 with ASCII.
        let chunks = chunk_text("abc", 1);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], "a");
        assert_eq!(chunks[1], "b");
        assert_eq!(chunks[2], "c");
    }

    #[test]
    fn chunk_text_many_paragraphs() {
        let text = (0..10)
            .map(|i| format!("paragraph {i}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let chunks = chunk_text(&text, 30);
        // Each paragraph is ~13 chars, so ~2 per chunk.
        assert!(chunks.len() >= 5);
        // Reassemble should preserve all paragraphs.
        let joined: String = chunks.join("");
        for i in 0..10 {
            assert!(joined.contains(&format!("paragraph {i}")));
        }
    }

    #[test]
    fn chunk_text_trailing_newlines() {
        let text = "first part\n\n\n\nsecond part";
        let chunks = chunk_text(&text, 15);
        assert!(chunks.len() >= 2);
        assert!(chunks[0].contains("first"));
        assert!(chunks.last().unwrap().contains("second"));
    }

    #[test]
    fn chunk_text_only_newlines() {
        let text = "\n\n\n\n\n";
        let chunks = chunk_text(&text, 3);
        // Should not produce empty chunks forever.
        assert!(!chunks.is_empty());
    }

    #[test]
    fn chunk_text_emoji_safe() {
        // Emoji are 4 bytes — boundary must not split them.
        let text = "🎉".repeat(50); // 200 bytes
        let chunks = chunk_text(&text, 50);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            // Valid UTF-8 and non-empty.
            assert!(!chunk.is_empty());
        }
    }

    #[test]
    fn find_split_at_newline() {
        let text = "hello\nworld and more";
        let point = find_split_point(text, 15, "\n");
        assert_eq!(point, Some(6)); // After "hello\n"
    }

    #[test]
    fn find_split_last_occurrence() {
        // rfind should find the *last* occurrence before boundary.
        // "a\n\nb\n\nc" = bytes: a(0) \n(1) \n(2) b(3) \n(4) \n(5) c(6)
        // Searching [..6] = "a\n\nb\n\n", last "\n\n" is at index 4, + 2 = 6.
        let text = "a\n\nb\n\nc";
        let point = find_split_point(text, 6, "\n\n");
        assert_eq!(point, Some(6));
    }
}
