//! Glob tool — finds files matching a glob pattern.

use std::fmt::Write;

use crate::{BuiltinTool, ToolContext, ToolError, ToolResult};
use serde_json::Value;
use std::path::PathBuf;
use std::time::SystemTime;
use walkdir::WalkDir;

/// Built-in tool for finding files by glob pattern.
pub struct GlobTool;

#[async_trait::async_trait]
impl BuiltinTool for GlobTool {
    fn name(&self) -> &'static str {
        "glob"
    }

    fn description(&self) -> &'static str {
        "Finds files matching a glob pattern (e.g. \"**/*.rs\", \"src/**/*.ts\"). \
         Returns matching file paths sorted by modification time (most recent first)."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files against"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to workspace root)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let pattern = args
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArguments("pattern is required".into()))?;

        let search_dir = args
            .get("path")
            .and_then(Value::as_str)
            .map_or_else(|| ctx.workspace_root.clone(), PathBuf::from);

        if !search_dir.exists() {
            return Err(ToolError::PathNotFound(search_dir.display().to_string()));
        }

        // Canonicalize to handle symlinks (e.g. /var -> /private/var on macOS)
        let search_dir = search_dir.canonicalize()?;

        let glob = globset::GlobBuilder::new(pattern)
            .literal_separator(false)
            .build()
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid glob pattern: {e}")))?
            .compile_matcher();

        // Walk the directory and collect matches
        let mut matches: Vec<(PathBuf, SystemTime)> = Vec::new();

        for entry in WalkDir::new(&search_dir)
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

            if entry.file_type().is_dir() {
                continue;
            }

            // Match against relative path from search dir
            let rel_path = entry
                .path()
                .strip_prefix(&search_dir)
                .unwrap_or(entry.path());

            if glob.is_match(rel_path) {
                let mtime = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                matches.push((entry.path().to_path_buf(), mtime));
            }
        }

        // Sort by mtime (most recent first)
        matches.sort_by(|a, b| b.1.cmp(&a.1));

        if matches.is_empty() {
            return Ok(format!("No files matching \"{pattern}\" found"));
        }

        let mut output = String::new();
        for (path, _) in &matches {
            output.push_str(&path.display().to_string());
            output.push('\n');
        }

        let _ = write!(output, "\n({} files matched)", matches.len());
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ctx_with_root(root: &std::path::Path) -> ToolContext {
        ToolContext::new(root.to_path_buf())
    }

    #[tokio::test]
    async fn test_glob_basic() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main(){}").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn test(){}").unwrap();
        std::fs::write(dir.path().join("c.txt"), "hello").unwrap();

        let ctx = ctx_with_root(dir.path());
        let result = GlobTool
            .execute(serde_json::json!({"pattern": "*.rs"}), &ctx)
            .await
            .unwrap();

        assert!(result.contains("a.rs"));
        assert!(result.contains("b.rs"));
        assert!(!result.contains("c.txt"));
        assert!(result.contains("2 files matched"));
    }

    #[tokio::test]
    async fn test_glob_recursive() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src").join("sub")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "").unwrap();
        std::fs::write(dir.path().join("src/sub/lib.rs"), "").unwrap();

        let ctx = ctx_with_root(dir.path());
        let result = GlobTool
            .execute(serde_json::json!({"pattern": "**/*.rs"}), &ctx)
            .await
            .unwrap();

        assert!(result.contains("main.rs"));
        assert!(result.contains("lib.rs"));
    }

    #[tokio::test]
    async fn test_glob_no_matches() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();

        let ctx = ctx_with_root(dir.path());
        let result = GlobTool
            .execute(serde_json::json!({"pattern": "*.rs"}), &ctx)
            .await
            .unwrap();

        assert!(result.contains("No files matching"));
    }

    #[tokio::test]
    async fn test_glob_invalid_pattern() {
        let ctx = ctx_with_root(&std::env::temp_dir());
        let result = GlobTool
            .execute(serde_json::json!({"pattern": "[invalid"}), &ctx)
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_glob_skips_hidden_dirs() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config"), "").unwrap();
        std::fs::write(dir.path().join("visible.rs"), "").unwrap();

        let ctx = ctx_with_root(dir.path());
        let result = GlobTool
            .execute(serde_json::json!({"pattern": "**/*"}), &ctx)
            .await
            .unwrap();

        assert!(result.contains("visible.rs"));
        assert!(!result.contains(".git"));
    }
}
