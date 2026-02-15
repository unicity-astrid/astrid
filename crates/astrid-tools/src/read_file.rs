//! Read file tool — reads a file with line numbers (cat -n style).

use std::fmt::Write;

use crate::{BuiltinTool, ToolContext, ToolError, ToolResult};
use serde_json::Value;

/// Default maximum lines to read.
const DEFAULT_LINE_LIMIT: usize = 2000;
/// Maximum line length before truncation.
const MAX_LINE_LENGTH: usize = 2000;

/// Built-in tool for reading files.
pub struct ReadFileTool;

#[async_trait::async_trait]
impl BuiltinTool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Reads a file from the filesystem. Returns contents with line numbers (cat -n format). \
         Default reads up to 2000 lines. Use offset and limit for large files. \
         Lines longer than 2000 characters are truncated."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (1-based). Only provide for large files."
                },
                "limit": {
                    "type": "integer",
                    "description": "Number of lines to read. Only provide for large files."
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let file_path = args
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArguments("file_path is required".into()))?;

        let offset = args
            .get("offset")
            .and_then(Value::as_u64)
            .map(|v| usize::try_from(v).unwrap_or(usize::MAX));

        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_LINE_LIMIT, |v| {
                usize::try_from(v).unwrap_or(usize::MAX)
            });

        let path = std::path::Path::new(file_path);
        if !path.exists() {
            return Err(ToolError::PathNotFound(file_path.to_string()));
        }

        // Binary detection: read first 8KB and check for null bytes
        let raw = tokio::fs::read(path).await?;
        let check_len = raw.len().min(8192);
        if raw[..check_len].contains(&0) {
            return Err(ToolError::ExecutionFailed(format!(
                "{file_path} appears to be a binary file"
            )));
        }

        let content = String::from_utf8(raw)
            .map_err(|_| ToolError::ExecutionFailed(format!("{file_path} is not valid UTF-8")))?;

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        // Apply offset (1-based)
        let start = offset.map_or(0, |o| o.saturating_sub(1));
        let end = start.saturating_add(limit).min(total_lines);

        if start >= total_lines {
            return Ok(format!(
                "(file has {total_lines} lines, offset {start} is past end)"
            ));
        }

        let mut output = String::new();
        for (idx, &line) in lines[start..end].iter().enumerate() {
            // Safety: start and idx are bounded by total_lines, +1 for 1-based display
            #[allow(clippy::arithmetic_side_effects)]
            let line_num = start + idx + 1;
            let display_line = if line.len() > MAX_LINE_LENGTH {
                &line[..MAX_LINE_LENGTH]
            } else {
                line
            };
            let _ = writeln!(output, "{line_num:>6}\t{display_line}");
        }

        if end < total_lines {
            let _ = write!(
                output,
                "\n(showing lines {}-{} of {total_lines}; use offset/limit for more)",
                start.saturating_add(1),
                end
            );
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;
    use tempfile::NamedTempFile;

    fn ctx() -> ToolContext {
        ToolContext::new(std::env::temp_dir())
    }

    #[tokio::test]
    async fn test_read_file_basic() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "line one").unwrap();
        writeln!(f, "line two").unwrap();
        writeln!(f, "line three").unwrap();

        let result = ReadFileTool
            .execute(
                serde_json::json!({"file_path": f.path().to_str().unwrap()}),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(result.contains("line one"));
        assert!(result.contains("line two"));
        assert!(result.contains("line three"));
        assert!(result.contains("     1\t"));
        assert!(result.contains("     2\t"));
        assert!(result.contains("     3\t"));
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let result = ReadFileTool
            .execute(
                serde_json::json!({"file_path": "/tmp/astrid_nonexistent_12345.txt"}),
                &ctx(),
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolError::PathNotFound(_)));
    }

    #[tokio::test]
    async fn test_read_file_with_offset_and_limit() {
        let mut f = NamedTempFile::new().unwrap();
        for i in 1..=20 {
            writeln!(f, "line {i}").unwrap();
        }

        let result = ReadFileTool
            .execute(
                serde_json::json!({
                    "file_path": f.path().to_str().unwrap(),
                    "offset": 5,
                    "limit": 3
                }),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(result.contains("     5\t"));
        assert!(result.contains("line 5"));
        assert!(result.contains("line 7"));
        assert!(!result.contains("line 8"));
    }

    #[tokio::test]
    async fn test_read_binary_file() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&[0x00, 0x01, 0x02, 0xFF]).unwrap();

        let result = ReadFileTool
            .execute(
                serde_json::json!({"file_path": f.path().to_str().unwrap()}),
                &ctx(),
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("binary file"));
    }

    #[tokio::test]
    async fn test_read_file_missing_arg() {
        let result = ReadFileTool.execute(serde_json::json!({}), &ctx()).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::InvalidArguments(_)
        ));
    }
}
