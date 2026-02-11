//! Grep tool — searches file contents with regex.

use std::fmt::Write;

use crate::{BuiltinTool, ToolContext, ToolError, ToolResult};
use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Maximum number of matching files to report.
const MAX_MATCHING_FILES: usize = 100;

/// Built-in tool for searching file contents.
pub struct GrepTool;

#[async_trait::async_trait]
impl BuiltinTool for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn description(&self) -> &'static str {
        "Searches file contents using regex. Supports context lines and file type filtering. \
         Returns matching lines in file:line:content format."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in (defaults to workspace root)"
                },
                "glob": {
                    "type": "string",
                    "description": "Glob to filter files (e.g. \"*.rs\", \"*.{ts,tsx}\")"
                },
                "context": {
                    "type": "integer",
                    "description": "Number of context lines to show before and after matches"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Case insensitive search (default: false)"
                }
            },
            "required": ["pattern"]
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let pattern_str = args
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArguments("pattern is required".into()))?;

        let case_insensitive = args
            .get("case_insensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let regex_pattern = if case_insensitive {
            format!("(?i){pattern_str}")
        } else {
            pattern_str.to_string()
        };

        let regex = Regex::new(&regex_pattern)
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid regex: {e}")))?;

        let search_path = args
            .get("path")
            .and_then(Value::as_str)
            .map_or_else(|| ctx.workspace_root.clone(), PathBuf::from);

        if !search_path.exists() {
            return Err(ToolError::PathNotFound(search_path.display().to_string()));
        }

        // Canonicalize to handle symlinks (e.g. /var -> /private/var on macOS)
        let search_path = search_path.canonicalize()?;

        let context_lines = args
            .get("context")
            .and_then(Value::as_u64)
            .map_or(0, |v| usize::try_from(v).unwrap_or(0));

        let file_glob = args
            .get("glob")
            .and_then(Value::as_str)
            .map(|g| {
                globset::GlobBuilder::new(g)
                    .literal_separator(false)
                    .build()
                    .map(|gb| gb.compile_matcher())
            })
            .transpose()
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid file glob: {e}")))?;

        // If search_path is a file, just search that file
        if search_path.is_file() {
            return search_file(&search_path, &regex, context_lines);
        }

        // Walk directory
        let mut output = String::new();
        let mut match_count = 0;
        let mut file_count = 0;

        for entry in WalkDir::new(&search_path)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                // Skip hidden directories (but not the root entry)
                if e.depth() == 0 {
                    return true;
                }
                e.file_name().to_str().is_none_or(|s| !s.starts_with('.'))
            })
        {
            let Ok(entry) = entry else { continue };

            if !entry.file_type().is_file() {
                continue;
            }

            // Apply file glob filter
            if let Some(ref glob) = file_glob {
                let rel = entry
                    .path()
                    .strip_prefix(&search_path)
                    .unwrap_or(entry.path());
                let file_name = entry.file_name().to_string_lossy();
                if !glob.is_match(rel) && !glob.is_match(file_name.as_ref()) {
                    continue;
                }
            }

            // Skip binary files (check first 512 bytes)
            if let Ok(data) = std::fs::read(entry.path()) {
                let check_len = data.len().min(512);
                if data[..check_len].contains(&0) {
                    continue;
                }
            }

            let Ok(content) = std::fs::read_to_string(entry.path()) else {
                continue;
            };

            let lines: Vec<&str> = content.lines().collect();
            let mut file_has_match = false;

            for (idx, line) in lines.iter().enumerate() {
                if regex.is_match(line) {
                    if !file_has_match {
                        file_has_match = true;
                        file_count += 1;
                        if file_count > MAX_MATCHING_FILES {
                            let _ = write!(
                                output,
                                "\n(stopped after {MAX_MATCHING_FILES} files with matches)"
                            );
                            return Ok(output);
                        }
                    }

                    match_count += 1;
                    write_context_lines(&mut output, entry.path(), &lines, idx, context_lines);
                }
            }
        }

        if match_count == 0 {
            return Ok(format!("No matches for \"{pattern_str}\" found"));
        }

        let _ = write!(output, "\n({match_count} matches in {file_count} files)");
        Ok(output)
    }
}

/// Write a match with context lines to the output buffer.
fn write_context_lines(
    output: &mut String,
    path: &Path,
    lines: &[&str],
    idx: usize,
    context: usize,
) {
    let line_num = idx + 1;

    // Context before
    let start = idx.saturating_sub(context);
    for (i, line) in lines[start..idx].iter().enumerate() {
        let _ = writeln!(output, "{}:{}-{}", path.display(), start + i + 1, line);
    }

    // The match itself
    let _ = writeln!(output, "{}:{line_num}:{}", path.display(), lines[idx]);

    // Context after
    let end = (idx + 1 + context).min(lines.len());
    for (i, line) in lines[(idx + 1)..end].iter().enumerate() {
        let _ = writeln!(output, "{}:{}-{}", path.display(), idx + 2 + i, line);
    }
}

/// Search a single file for matches.
fn search_file(path: &Path, regex: &Regex, context_lines: usize) -> ToolResult {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();
    let mut output = String::new();
    let mut match_count = 0;

    for (idx, line) in lines.iter().enumerate() {
        if regex.is_match(line) {
            match_count += 1;
            write_context_lines(&mut output, path, &lines, idx, context_lines);
        }
    }

    if match_count == 0 {
        return Ok(format!("No matches found in {}", path.display()));
    }

    let _ = write!(output, "\n({match_count} matches)");
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;
    use tempfile::{NamedTempFile, TempDir};

    fn ctx_with_root(root: &std::path::Path) -> ToolContext {
        ToolContext::new(root.to_path_buf())
    }

    #[tokio::test]
    async fn test_grep_basic() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}\nfn test() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn helper() {}\n").unwrap();

        let ctx = ctx_with_root(dir.path());
        let result = GrepTool
            .execute(serde_json::json!({"pattern": "fn main"}), &ctx)
            .await
            .unwrap();

        assert!(result.contains("fn main"));
        assert!(result.contains("1 matches"));
    }

    #[tokio::test]
    async fn test_grep_with_glob_filter() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "fn main() {}\n").unwrap();

        let ctx = ctx_with_root(dir.path());
        let result = GrepTool
            .execute(
                serde_json::json!({"pattern": "fn main", "glob": "*.rs"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.contains("a.rs"));
        assert!(!result.contains("b.txt"));
    }

    #[tokio::test]
    async fn test_grep_case_insensitive() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.txt"), "Hello World\nhello world\n").unwrap();

        let ctx = ctx_with_root(dir.path());
        let result = GrepTool
            .execute(
                serde_json::json!({"pattern": "hello", "case_insensitive": true}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.contains("Hello World"));
        assert!(result.contains("hello world"));
    }

    #[tokio::test]
    async fn test_grep_no_matches() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.txt"), "hello world\n").unwrap();

        let ctx = ctx_with_root(dir.path());
        let result = GrepTool
            .execute(serde_json::json!({"pattern": "foobar"}), &ctx)
            .await
            .unwrap();

        assert!(result.contains("No matches"));
    }

    #[tokio::test]
    async fn test_grep_context_lines() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "line 1").unwrap();
        writeln!(f, "line 2").unwrap();
        writeln!(f, "MATCH").unwrap();
        writeln!(f, "line 4").unwrap();
        writeln!(f, "line 5").unwrap();

        let ctx = ctx_with_root(&std::env::temp_dir());
        let result = GrepTool
            .execute(
                serde_json::json!({
                    "pattern": "MATCH",
                    "path": f.path().to_str().unwrap(),
                    "context": 1
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.contains("line 2"));
        assert!(result.contains("MATCH"));
        assert!(result.contains("line 4"));
    }

    #[tokio::test]
    async fn test_grep_single_file() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "hello").unwrap();
        writeln!(f, "world").unwrap();

        let ctx = ctx_with_root(&std::env::temp_dir());
        let result = GrepTool
            .execute(
                serde_json::json!({
                    "pattern": "hello",
                    "path": f.path().to_str().unwrap()
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.contains("hello"));
        assert!(result.contains("1 matches"));
    }

    #[tokio::test]
    async fn test_grep_invalid_regex() {
        let ctx = ctx_with_root(&std::env::temp_dir());
        let result = GrepTool
            .execute(serde_json::json!({"pattern": "[invalid"}), &ctx)
            .await;

        assert!(result.is_err());
    }
}
