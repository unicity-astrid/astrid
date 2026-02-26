//! Markdown formatting and message chunking for Discord.
//!
//! Discord natively supports a subset of markdown, so LLM output
//! requires only light sanitization. The main concern is the 2000-char
//! message limit.

/// Discord's maximum message content length.
const DISCORD_MAX_LEN: usize = 2000;

/// Target chunk size, leaving headroom for code fence continuations.
const TARGET_CHUNK_LEN: usize = 1900;

/// Sanitize LLM markdown output for Discord.
///
/// - Strips raw HTML tags (Discord does not render them)
/// - Escapes accidental `@everyone`, `@here`, user/role mentions
/// - Ensures code blocks are properly closed
pub(crate) fn sanitize_for_discord(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut chars = md.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            // Skip HTML tags: consume until '>'.
            let mut is_tag = false;
            let mut tag_buf = String::new();
            for inner in chars.by_ref() {
                if inner == '>' {
                    is_tag = true;
                    break;
                }
                tag_buf.push(inner);
                // Don't consume too much if it's not really a tag.
                if tag_buf.len() > 50 {
                    break;
                }
            }
            if !is_tag {
                // Not a tag — emit what we consumed.
                out.push('<');
                out.push_str(&tag_buf);
            }
        } else if ch == '@' {
            // Escape @everyone and @here.
            // Collect alphabetic chars without consuming the terminator.
            let mut word = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_alphabetic() {
                    word.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            if word == "everyone" || word == "here" {
                out.push('@');
                out.push('\u{200B}'); // zero-width space
                out.push_str(&word);
            } else {
                out.push('@');
                out.push_str(&word);
            }
        } else {
            out.push(ch);
        }
    }

    // Ensure code blocks are properly closed.
    let fence_count = out.matches("```").count();
    if !fence_count.is_multiple_of(2) {
        out.push_str("\n```");
    }

    out
}

/// Split text into chunks that fit within Discord's 2000-char limit.
///
/// Handles code block continuation: if a chunk ends inside a fenced
/// code block, the block is closed and re-opened in the next chunk.
///
/// Split priority: paragraph (`\n\n`) > newline (`\n`) > hard cut.
pub(crate) fn chunk_discord(text: &str, max_len: usize) -> Vec<String> {
    let max_len = if max_len == 0 {
        TARGET_CHUNK_LEN
    } else {
        max_len.min(DISCORD_MAX_LEN)
    };

    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;
    let mut in_code_block = false;
    let mut code_lang = String::new();

    while !remaining.is_empty() {
        // If we're continuing a code block from the previous chunk,
        // prepend the opening fence.
        let prefix = if in_code_block {
            format!("```{code_lang}\n")
        } else {
            String::new()
        };

        let budget = max_len.saturating_sub(prefix.len());

        if remaining.len() <= budget {
            let mut chunk = prefix;
            chunk.push_str(remaining);
            chunks.push(chunk);
            break;
        }

        // Find split point.
        let hard_cut = floor_char_boundary(remaining, budget);
        let split_at = find_split_point(remaining, hard_cut, "\n\n")
            .or_else(|| find_split_point(remaining, hard_cut, "\n"))
            .unwrap_or(hard_cut);

        let (chunk_text, rest) = remaining.split_at(split_at);

        // Track code block state within this chunk.
        let mut chunk_in_block = in_code_block;
        let mut chunk_lang = code_lang.clone();
        for line in chunk_text.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") {
                if chunk_in_block {
                    chunk_in_block = false;
                    chunk_lang.clear();
                } else {
                    chunk_in_block = true;
                    chunk_lang = trimmed.trim_start_matches('`').trim().to_string();
                }
            }
        }

        let mut chunk = prefix;
        chunk.push_str(chunk_text);

        // If we end inside a code block, close it.
        if chunk_in_block {
            chunk.push_str("\n```");
        }

        chunks.push(chunk);

        // Update state for next iteration.
        in_code_block = chunk_in_block;
        code_lang = chunk_lang;
        remaining = rest.trim_start_matches('\n');
    }

    chunks
}

/// Find the largest byte index ≤ `i` that is a char boundary.
pub(crate) fn floor_char_boundary(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    let mut pos = i;
    while pos > 0 && !s.is_char_boundary(pos) {
        pos = pos.saturating_sub(1);
    }
    pos
}

/// Search backwards from `boundary` for `delimiter` in `text`.
fn find_split_point(text: &str, boundary: usize, delimiter: &str) -> Option<usize> {
    let region = &text[..boundary];
    region
        .rfind(delimiter)
        .map(|pos| pos.saturating_add(delimiter.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_html() {
        let input = "Hello <b>world</b>!";
        let out = sanitize_for_discord(input);
        assert_eq!(out, "Hello world!");
    }

    #[test]
    fn sanitize_escapes_everyone() {
        let input = "Hey @everyone look!";
        let out = sanitize_for_discord(input);
        assert!(out.contains("@\u{200B}everyone"));
    }

    #[test]
    fn sanitize_escapes_here() {
        let input = "@here please respond";
        let out = sanitize_for_discord(input);
        assert!(out.contains("@\u{200B}here"));
    }

    #[test]
    fn sanitize_preserves_chars_after_mention() {
        // Regression: take_while used to consume the non-alpha char.
        let input = "@everyone! wake up";
        let out = sanitize_for_discord(input);
        assert_eq!(out, "@\u{200B}everyone! wake up");
    }

    #[test]
    fn sanitize_at_non_keyword() {
        let input = "@user123 hello";
        let out = sanitize_for_discord(input);
        // "user" is alphabetic, "123" is not, so word = "user"
        // then "123 hello" continues normally.
        assert_eq!(out, "@user123 hello");
    }

    #[test]
    fn sanitize_closes_unclosed_code_block() {
        let input = "```rust\nfn main() {}";
        let out = sanitize_for_discord(input);
        assert!(out.ends_with("```"));
        assert_eq!(out.matches("```").count(), 2);
    }

    #[test]
    fn sanitize_leaves_closed_code_blocks() {
        let input = "```\ncode\n```";
        let out = sanitize_for_discord(input);
        assert_eq!(out.matches("```").count(), 2);
    }

    #[test]
    fn chunk_short_message() {
        let chunks = chunk_discord("short", 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "short");
    }

    #[test]
    fn chunk_splits_at_paragraph() {
        let text = format!("{}\n\n{}", "a".repeat(50), "b".repeat(50));
        let chunks = chunk_discord(&text, 60);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].starts_with('a'));
        assert!(chunks[1].starts_with('b'));
    }

    #[test]
    fn chunk_hard_splits_no_breaks() {
        let text = "x".repeat(200);
        let chunks = chunk_discord(&text, 100);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.len() <= 100);
        }
    }

    #[test]
    fn chunk_continues_code_block() {
        let text = format!("```rust\n{}\n```", "x".repeat(300));
        let chunks = chunk_discord(&text, 200);
        assert!(chunks.len() >= 2);
        // First chunk should close the code block.
        assert!(chunks[0].ends_with("```"));
        // Second chunk should re-open it.
        assert!(chunks[1].starts_with("```"));
    }

    #[test]
    fn chunk_empty_string() {
        let chunks = chunk_discord("", 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "");
    }

    #[test]
    fn chunk_respects_discord_max() {
        let text = "x".repeat(5000);
        let chunks = chunk_discord(&text, 2000);
        for chunk in &chunks {
            assert!(chunk.len() <= 2000);
        }
    }

    // --- sanitize edge cases ---

    #[test]
    fn sanitize_preserves_normal_text() {
        let input = "Hello, world! How are you?";
        assert_eq!(sanitize_for_discord(input), input);
    }

    #[test]
    fn sanitize_preserves_discord_markdown() {
        let input = "**bold** *italic* __underline__ ~~strike~~ `code`";
        assert_eq!(sanitize_for_discord(input), input);
    }

    #[test]
    fn sanitize_strips_multiple_html_tags() {
        let input = "<p>Hello</p> <br/> <a href='x'>link</a>";
        let out = sanitize_for_discord(input);
        assert!(!out.contains("</"));
        assert!(out.contains("Hello"));
        assert!(out.contains("link"));
    }

    #[test]
    fn sanitize_long_angle_bracket_not_tag() {
        // A '<' followed by 50+ chars without '>' is not treated as tag.
        let long = format!("<{}", "x".repeat(60));
        let out = sanitize_for_discord(&long);
        assert!(out.starts_with('<'));
    }

    #[test]
    fn sanitize_at_normal_mention_unchanged() {
        let input = "Hello @someuser!";
        let out = sanitize_for_discord(input);
        assert!(out.contains("@someuser"));
        assert!(!out.contains('\u{200B}'));
    }

    #[test]
    fn sanitize_multiple_everyone_escaped() {
        let input = "@everyone @everyone @here";
        let out = sanitize_for_discord(input);
        assert_eq!(out.matches("@\u{200B}").count(), 3);
    }

    #[test]
    fn sanitize_already_closed_code_block_untouched() {
        let input = "```\nfoo\n```\n```\nbar\n```";
        let out = sanitize_for_discord(input);
        assert_eq!(out.matches("```").count(), 4);
    }

    #[test]
    fn sanitize_empty_string() {
        assert_eq!(sanitize_for_discord(""), "");
    }

    #[test]
    fn sanitize_unicode_preserved() {
        let input = "こんにちは 🌍 привет";
        let out = sanitize_for_discord(input);
        assert_eq!(out, input);
    }

    // --- chunk edge cases ---

    #[test]
    fn chunk_exact_limit_no_split() {
        let text = "x".repeat(100);
        let chunks = chunk_discord(&text, 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 100);
    }

    #[test]
    fn chunk_one_over_limit() {
        let text = "x".repeat(101);
        let chunks = chunk_discord(&text, 100);
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn chunk_multibyte_safe() {
        let text = "\u{3042}".repeat(100); // 300 bytes
        let chunks = chunk_discord(&text, 50);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(!chunk.is_empty());
        }
    }

    #[test]
    fn chunk_code_block_lang_preserved() {
        let code = "x\n".repeat(200);
        let text = format!("```python\n{code}```");
        let chunks = chunk_discord(&text, 100);
        assert!(chunks.len() >= 2);
        assert!(chunks[0].contains("```python"));
        if chunks[1].starts_with("```") {
            assert!(chunks[1].starts_with("```python"));
        }
    }

    #[test]
    fn chunk_multiple_code_blocks() {
        let text = "```\nblock1\n```\n\nSome text\n\n```rust\nblock2\n```";
        let chunks = chunk_discord(&text, 30);
        for chunk in &chunks {
            let fences = chunk.matches("```").count();
            assert_eq!(fences % 2, 0, "Unbalanced fences in chunk: {chunk:?}");
        }
    }

    #[test]
    fn chunk_preserves_all_content() {
        let text = format!(
            "{}\n\n{}\n\n{}",
            "a".repeat(100),
            "b".repeat(100),
            "c".repeat(100)
        );
        let chunks = chunk_discord(&text, 150);
        let joined: String = chunks.join("");
        assert!(joined.contains(&"a".repeat(100)));
        assert!(joined.contains(&"b".repeat(100)));
        assert!(joined.contains(&"c".repeat(100)));
    }

    #[test]
    fn chunk_zero_max_uses_default_target() {
        let text = "x".repeat(1899);
        let chunks = chunk_discord(&text, 0);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn chunk_max_capped_at_discord_limit() {
        let text = "x".repeat(5000);
        let chunks = chunk_discord(&text, 3000);
        for chunk in &chunks {
            assert!(chunk.len() <= 2000);
        }
    }

    // --- floor_char_boundary ---

    #[test]
    fn floor_char_boundary_ascii() {
        assert_eq!(floor_char_boundary("hello", 3), 3);
    }

    #[test]
    fn floor_char_boundary_multibyte() {
        let s = "a\u{3042}b"; // "a" + 3-byte char + "b"
        assert_eq!(floor_char_boundary(s, 2), 1);
        assert_eq!(floor_char_boundary(s, 4), 4);
    }

    #[test]
    fn floor_char_boundary_beyond_len() {
        assert_eq!(floor_char_boundary("abc", 100), 3);
    }

    #[test]
    fn floor_char_boundary_zero() {
        assert_eq!(floor_char_boundary("abc", 0), 0);
    }
}
