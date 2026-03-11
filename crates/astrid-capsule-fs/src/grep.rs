//! Pure grep matching logic, separated from VFS for testability.

/// Maximum number of matching lines returned by `grep_search`.
pub(crate) const GREP_MAX_MATCHES: usize = 100;
/// Maximum number of files visited during a recursive grep walk.
pub(crate) const GREP_MAX_FILES: usize = 1_000;
/// Maximum directory recursion depth for grep walks.
pub(crate) const GREP_MAX_DEPTH: usize = 20;

/// Scans `content` line-by-line for `pattern`, appending
/// `path:line_number:line` to `matches`.
pub(crate) fn grep_content(path: &str, content: &str, pattern: &str, matches: &mut Vec<String>) {
    for (i, line) in content.lines().enumerate() {
        if matches.len() >= GREP_MAX_MATCHES {
            return;
        }
        if line.contains(pattern) {
            matches.push(format!("{path}:{}:{line}", i + 1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_match() {
        let mut matches = Vec::new();
        grep_content("test.rs", "hello foo bar\nbaz\nfoo again", "foo", &mut matches);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], "test.rs:1:hello foo bar");
        assert_eq!(matches[1], "test.rs:3:foo again");
    }

    #[test]
    fn no_match() {
        let mut matches = Vec::new();
        grep_content("test.rs", "hello\nworld\n", "xyz", &mut matches);
        assert!(matches.is_empty());
    }

    #[test]
    fn first_and_last_line() {
        let mut matches = Vec::new();
        grep_content("f.txt", "hit first\nmiss\nhit last", "hit", &mut matches);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], "f.txt:1:hit first");
        assert_eq!(matches[1], "f.txt:3:hit last");
    }

    #[test]
    fn respects_max_matches() {
        let mut matches = Vec::new();
        for i in 0..GREP_MAX_MATCHES - 1 {
            matches.push(format!("pre:{i}:filler"));
        }
        grep_content("f.txt", "x\nx\nx\n", "x", &mut matches);
        assert_eq!(matches.len(), GREP_MAX_MATCHES);
        assert_eq!(matches[GREP_MAX_MATCHES - 1], "f.txt:1:x");
    }

    #[test]
    fn literal_regex_chars() {
        let mut matches = Vec::new();
        grep_content("f.txt", "literal .* here\nno match", ".*", &mut matches);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], "f.txt:1:literal .* here");
    }

    #[test]
    fn dot_is_literal() {
        let mut matches = Vec::new();
        grep_content("f.txt", "has a dot.\nno dot here? yes", ".", &mut matches);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], "f.txt:1:has a dot.");
    }

    #[test]
    fn empty_content() {
        let mut matches = Vec::new();
        grep_content("f.txt", "", "x", &mut matches);
        assert!(matches.is_empty());
    }

    #[test]
    fn single_line_no_newline() {
        let mut matches = Vec::new();
        grep_content("f.txt", "hello world", "hello", &mut matches);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], "f.txt:1:hello world");
    }
}
