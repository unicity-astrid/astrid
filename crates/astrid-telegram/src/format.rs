//! Markdown to Telegram HTML conversion and text chunking.

use regex::Regex;
use std::sync::LazyLock;

/// Maximum message length for Telegram (with some margin below 4096).
const MAX_MESSAGE_LEN: usize = 4000;

/// Escape text for safe inclusion in Telegram HTML.
///
/// Escapes `&`, `<`, `>`, `"`, and `'` so the output is safe in both text
/// content and HTML attributes (e.g. `href="..."`).
pub fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Convert LLM markdown to Telegram HTML.
///
/// Handles: bold, italic, inline code, code blocks, links.
/// Telegram supports a limited subset of HTML: `<b>`, `<i>`, `<code>`,
/// `<pre>`, `<a href="...">`.
pub fn md_to_telegram_html(md: &str) -> String {
    static CODE_BLOCK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"```(\w*)\n?([\s\S]*?)```").expect("invalid regex"));
    static INLINE_CODE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"`([^`]+)`").expect("invalid regex"));
    static BOLD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*").expect("invalid regex"));
    // Simple italic: single * not preceded/followed by *. We capture the
    // surrounding characters so we can preserve them in the replacement.
    static ITALIC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"([^*]|^)\*([^*]+)\*([^*]|$)").expect("invalid regex"));
    static LINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").expect("invalid regex"));
    static HEADING: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^#{1,6}\s+(.+)$").expect("invalid regex"));

    // First pass: extract code blocks to protect them from other transforms.
    let mut protected: Vec<String> = Vec::new();
    let text = CODE_BLOCK.replace_all(md, |caps: &regex::Captures<'_>| {
        let lang = &caps[1];
        let code = html_escape(&caps[2]);
        let placeholder = format!("\x00CODE{}\x00", protected.len());
        // Telegram HTML only supports <pre> and <pre><code>...</code></pre>,
        // not arbitrary attributes like class="language-...".
        let _ = lang; // language info not usable in Telegram HTML
        protected.push(format!("<pre>{code}</pre>"));
        placeholder
    });

    // Second pass: extract inline code so it's protected from bold/italic.
    let text = INLINE_CODE.replace_all(&text, |caps: &regex::Captures<'_>| {
        let code = html_escape(&caps[1]);
        let placeholder = format!("\x00CODE{}\x00", protected.len());
        protected.push(format!("<code>{code}</code>"));
        placeholder
    });

    // Escape HTML in non-code sections. Placeholders (\x00CODE{i}\x00) are
    // inert — they don't contain any characters that html_escape touches.
    let text = html_escape(&text);

    // Apply inline transforms while placeholders are still in the text,
    // so bold/italic/link/heading don't match inside code regions.
    let text = BOLD.replace_all(&text, "<b>$1</b>");
    let text = ITALIC.replace_all(&text, "$1<i>$2</i>$3");
    let text = LINK.replace_all(&text, |caps: &regex::Captures<'_>| {
        let label = &caps[1];
        let url = &caps[2];
        // Only allow safe URL schemes to prevent javascript:/data: injection.
        // Note: scheme check uses already-escaped text (& → &amp;), but the
        // prefixes we check don't contain escapable chars so this is safe.
        if url.starts_with("http://")
            || url.starts_with("https://")
            || url.starts_with("tg://")
            || url.starts_with("mailto:")
        {
            // `url` is already HTML-escaped by the earlier `html_escape(&text)`
            // call. Use it directly to avoid double-escaping `&` → `&amp;amp;`.
            format!("<a href=\"{url}\">{label}</a>")
        } else {
            // Render as plain text for unsafe schemes.
            format!("{label} ({url})")
        }
    });
    let text = HEADING.replace_all(&text, "<b>$1</b>");

    // Restore all protected regions (code blocks + inline code) now that
    // inline transforms are done.
    let mut text = text.into_owned();
    for (i, block) in protected.iter().enumerate() {
        let placeholder = format!("\x00CODE{i}\x00");
        text = text.replace(&placeholder, block);
    }

    text
}

/// Split text into chunks that fit within Telegram's message size limit.
///
/// Tries to split at paragraph boundaries first, then newlines, then
/// hard-cuts at `max_len`.
pub fn chunk_text(text: &str, max_len: usize) -> Vec<String> {
    let max_len = if max_len == 0 {
        MAX_MESSAGE_LEN
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
/// `boundary` must be a valid char boundary in `text`.
fn find_split_point(text: &str, boundary: usize, delimiter: &str) -> Option<usize> {
    let search_region = &text[..boundary];
    search_region
        .rfind(delimiter)
        // Safety: pos from rfind() is within bounds, delimiter.len() keeps us within text
        .map(|pos| pos.saturating_add(delimiter.len()))
}

/// Find a safe truncation boundary in an HTML string at or before `max_len`.
///
/// Walks backwards from the char boundary until we're not inside a tag or
/// entity.
pub(crate) fn find_safe_html_boundary(html: &str, max_len: usize) -> usize {
    let mut boundary = html.floor_char_boundary(max_len.min(html.len()));

    while boundary > 0 {
        let bytes = &html.as_bytes()[..boundary];
        // Check if we're inside a tag: find the last '<' or '>'.
        let last_open = bytes.iter().rposition(|&b| b == b'<');
        let last_close = bytes.iter().rposition(|&b| b == b'>');
        let inside_tag = match (last_open, last_close) {
            (Some(lt), Some(gt)) => lt > gt,
            (Some(_), None) => true,
            _ => false,
        };
        // Check if we're inside an entity: find the last '&' or ';'.
        let last_amp = bytes.iter().rposition(|&b| b == b'&');
        let last_semi = bytes.iter().rposition(|&b| b == b';');
        let inside_entity = match (last_amp, last_semi) {
            (Some(amp), Some(semi)) => amp > semi,
            (Some(_), None) => true,
            _ => false,
        };

        if !inside_tag && !inside_entity {
            break;
        }
        // Walk back one char.
        boundary = html.floor_char_boundary(boundary.saturating_sub(1));
    }

    boundary
}

/// Close any unclosed HTML tags in a truncated HTML fragment.
///
/// Scans for open/close tags and appends closing tags for any that are still
/// open at the end. This prevents Telegram HTML parse errors when we truncate
/// in the middle of formatted text.
pub(crate) fn close_open_tags(html: &str) -> String {
    use std::fmt::Write as _;

    static TAG_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<(/?)(\w+)[^>]*>").expect("invalid regex"));

    let mut open_tags: Vec<String> = Vec::new();
    for cap in TAG_RE.captures_iter(html) {
        let is_close = &cap[1] == "/";
        let tag_name = cap[2].to_lowercase();
        if is_close {
            // Remove the most recent matching open tag.
            if let Some(pos) = open_tags.iter().rposition(|t| *t == tag_name) {
                open_tags.remove(pos);
            }
        } else {
            open_tags.push(tag_name);
        }
    }

    if open_tags.is_empty() {
        return html.to_string();
    }

    let mut result = html.to_string();
    for tag in open_tags.into_iter().rev() {
        let _ = write!(result, "</{tag}>");
    }
    result
}

/// Split HTML into chunks that fit within Telegram's message size limit.
///
/// Like [`chunk_text`] but HTML-aware: avoids splitting inside a tag (`<...>`)
/// or entity (`&...;`).
pub fn chunk_html(html: &str, max_len: usize) -> Vec<String> {
    // Reserve headroom for closing tags appended by close_open_tags.
    // Worst case: several nested tags like </pre></code></b></i></a> ≈ 35 bytes.
    const CLOSING_TAG_HEADROOM: usize = 50;

    let max_len = if max_len == 0 {
        MAX_MESSAGE_LEN
    } else {
        max_len
    };
    let split_limit = max_len.saturating_sub(CLOSING_TAG_HEADROOM);

    if html.len() <= max_len {
        return vec![html.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = html;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Find an HTML-safe hard-cut boundary within the reduced limit
        // so that closing tags don't push us over max_len.
        let hard_cut = find_safe_html_boundary(remaining, split_limit);
        // Prefer splitting at paragraph or newline boundaries within that
        // safe region.
        let split_at = find_split_point(remaining, hard_cut, "\n\n")
            .or_else(|| find_split_point(remaining, hard_cut, "\n"))
            .unwrap_or(hard_cut);

        // Guard against zero-progress (e.g. max_len <= CLOSING_TAG_HEADROOM).
        let split_at = if split_at == 0 {
            remaining.floor_char_boundary(max_len.max(1))
        } else {
            split_at
        };

        let (chunk, rest) = remaining.split_at(split_at);
        // Close any tags left open by the split so each chunk is valid
        // standalone HTML (Telegram rejects malformed HTML per message).
        chunks.push(close_open_tags(chunk));
        remaining = rest.trim_start_matches('\n');
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- html_escape ---

    #[test]
    fn html_escape_ampersand() {
        assert_eq!(html_escape("a & b"), "a &amp; b");
    }

    #[test]
    fn html_escape_angle_brackets() {
        assert_eq!(
            html_escape("<script>alert(1)</script>"),
            "&lt;script&gt;alert(1)&lt;/script&gt;"
        );
    }

    #[test]
    fn html_escape_all_special_chars() {
        assert_eq!(html_escape("<b>&test</b>"), "&lt;b&gt;&amp;test&lt;/b&gt;");
    }

    #[test]
    fn html_escape_quotes() {
        assert_eq!(html_escape(r#"a"b'c"#), "a&quot;b&#39;c");
    }

    #[test]
    fn html_escape_empty_string() {
        assert_eq!(html_escape(""), "");
    }

    #[test]
    fn html_escape_no_special_chars() {
        assert_eq!(html_escape("hello world"), "hello world");
    }

    // --- md_to_telegram_html: bold ---

    #[test]
    fn md_bold() {
        let result = md_to_telegram_html("Hello **world**");
        assert!(result.contains("<b>world</b>"));
    }

    #[test]
    fn md_bold_multiple() {
        let result = md_to_telegram_html("**a** and **b**");
        assert!(result.contains("<b>a</b>"));
        assert!(result.contains("<b>b</b>"));
    }

    // --- md_to_telegram_html: code blocks ---

    #[test]
    fn md_code_block_with_lang() {
        let result = md_to_telegram_html("```rust\nfn main() {}\n```");
        assert!(result.contains("<pre>"));
        assert!(result.contains("fn main()"));
        // Telegram doesn't support class attributes, so language is omitted.
        assert!(!result.contains("language-"));
    }

    #[test]
    fn md_code_block_without_lang() {
        let result = md_to_telegram_html("```\nhello\n```");
        assert!(result.contains("<pre>hello"));
        // Should not contain language class.
        assert!(!result.contains("language-"));
    }

    #[test]
    fn md_code_block_escapes_html() {
        let result = md_to_telegram_html("```\n<div>test</div>\n```");
        assert!(result.contains("&lt;div&gt;"));
    }

    // --- md_to_telegram_html: inline code ---

    #[test]
    fn md_inline_code() {
        let result = md_to_telegram_html("Use `cargo build` here");
        assert!(result.contains("<code>cargo build</code>"));
    }

    #[test]
    fn md_inline_code_preserves_content() {
        let result = md_to_telegram_html("Run `ls -la`");
        assert!(result.contains("<code>ls -la</code>"));
    }

    #[test]
    fn md_inline_code_not_affected_by_bold() {
        let result = md_to_telegram_html("Use `**not bold**` here");
        // The **...** inside backticks must NOT be converted to <b>.
        assert!(result.contains("<code>**not bold**</code>"));
        assert!(!result.contains("<b>not bold</b>"));
    }

    #[test]
    fn md_inline_code_escapes_html() {
        let result = md_to_telegram_html("Use `<div>` tag");
        assert!(result.contains("<code>&lt;div&gt;</code>"));
    }

    // --- md_to_telegram_html: links ---

    #[test]
    fn md_link() {
        let result = md_to_telegram_html("Visit [Google](https://google.com)");
        assert!(result.contains(r#"<a href="https://google.com">Google</a>"#));
    }

    #[test]
    fn md_link_unsafe_scheme_rejected() {
        let result = md_to_telegram_html("Click [here](javascript:alert(1))");
        // Should not produce an <a href> for javascript: URLs.
        assert!(!result.contains("<a href"));
        assert!(result.contains("here"));
    }

    // --- md_to_telegram_html: headings ---

    #[test]
    fn md_heading() {
        let result = md_to_telegram_html("# Title");
        assert!(result.contains("<b>Title</b>"));
    }

    #[test]
    fn md_heading_h3() {
        let result = md_to_telegram_html("### Subtitle");
        assert!(result.contains("<b>Subtitle</b>"));
    }

    // --- md_to_telegram_html: plain text ---

    #[test]
    fn md_plain_text_escapes_html() {
        let result = md_to_telegram_html("1 < 2 & 3 > 0");
        assert!(result.contains("&lt;"));
        assert!(result.contains("&amp;"));
        assert!(result.contains("&gt;"));
    }

    #[test]
    fn md_empty_string() {
        assert_eq!(md_to_telegram_html(""), "");
    }

    // --- chunk_text ---

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
        // 3-byte chars — a max_len that falls mid-char must not panic
        let text = "あ".repeat(100); // 300 bytes
        let chunks = chunk_text(&text, 50);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            // All chunks must be valid UTF-8 (implicit by being String)
            assert!(!chunk.is_empty());
        }
    }

    #[test]
    fn chunk_text_preserves_all_content() {
        let text = "a".repeat(150) + "\n\n" + &"b".repeat(150);
        let chunks = chunk_text(&text, 200);
        let reassembled: String = chunks.join("");
        // All content should be preserved (except possibly stripped newlines
        // between chunks).
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

    // --- find_safe_html_boundary ---

    #[test]
    fn html_boundary_avoids_mid_tag() {
        let html = "x".repeat(95) + "<b>bold</b>";
        // max_len=96 lands at position 96 which is inside "<b>" (the 'b').
        let b = find_safe_html_boundary(&html, 96);
        assert_eq!(b, 95); // Walks back to before the '<'
    }

    #[test]
    fn html_boundary_avoids_mid_entity() {
        let html = "x".repeat(95) + "&amp; rest";
        let b = find_safe_html_boundary(&html, 98);
        assert_eq!(b, 95); // Before the '&'
    }

    #[test]
    fn html_boundary_after_complete_tag() {
        let html = "x".repeat(90) + "<b>y</b>" + &"z".repeat(10);
        // Boundary at 102 lands in the 'z' region — not inside any tag.
        let b = find_safe_html_boundary(&html, 102);
        assert_eq!(b, 102);
    }

    // --- chunk_html ---

    #[test]
    fn chunk_html_short() {
        let chunks = chunk_html("<b>hello</b>", 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "<b>hello</b>");
    }

    #[test]
    fn chunk_html_does_not_split_mid_entity() {
        // Build HTML with lots of &amp; entities so total > limit.
        let html = "&amp; ".repeat(200); // 1200 bytes
        let chunks = chunk_html(&html, 100);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            // No chunk should end with a partial entity like "&am".
            assert!(
                !chunk.ends_with('&'),
                "chunk ends with partial entity: {chunk}"
            );
            assert!(
                !chunk.contains("&am\n") && !chunk.ends_with("&am"),
                "chunk has partial entity: {chunk}"
            );
        }
    }

    #[test]
    fn chunk_html_does_not_split_mid_tag() {
        let padding = "x".repeat(95);
        let html = format!("{padding}<b>bold</b>{padding}<i>ital</i>");
        let chunks = chunk_html(&html, 100);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            // No chunk should end with an unclosed '<'.
            let trimmed = chunk.trim_end();
            if let Some(last_lt) = trimmed.rfind('<') {
                // If there's a '<', there must be a '>' after it.
                assert!(
                    trimmed[last_lt..].contains('>'),
                    "chunk ends inside a tag: {chunk}"
                );
            }
        }
    }

    #[test]
    fn chunk_html_handles_expanded_entities() {
        // Simulate the critical bug: lots of &amp; that would blow past 4096
        // if we'd chunked the source markdown at 4000 first.
        let html = "&amp;".repeat(1000); // 5000 bytes
        let chunks = chunk_html(&html, 4000);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.len() <= 4000);
        }
    }

    #[test]
    fn chunk_html_prefers_newline_split() {
        // 200+2+200 = 402 bytes total, max_len=300 forces a split.
        // split_limit = 300-50 = 250 which covers the first 200-char part.
        let part_a = "a".repeat(200);
        let part_b = "b".repeat(200);
        let html = format!("{part_a}\n\n{part_b}");
        let chunks = chunk_html(&html, 300);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].starts_with('a'));
        assert!(chunks[1].starts_with('b'));
    }

    // --- close_open_tags ---

    #[test]
    fn close_open_tags_balanced() {
        assert_eq!(close_open_tags("<b>bold</b>"), "<b>bold</b>");
    }

    #[test]
    fn close_open_tags_unclosed_b() {
        assert_eq!(close_open_tags("<b>bold"), "<b>bold</b>");
    }

    #[test]
    fn close_open_tags_nested() {
        // <b><i>text → should close </i></b> in reverse order.
        assert_eq!(close_open_tags("<b><i>text"), "<b><i>text</i></b>");
    }

    #[test]
    fn close_open_tags_no_tags() {
        assert_eq!(close_open_tags("plain text"), "plain text");
    }

    #[test]
    fn close_open_tags_partial_close() {
        // <b><i>text</i> → only <b> is unclosed.
        assert_eq!(close_open_tags("<b><i>text</i>"), "<b><i>text</i></b>");
    }
}
