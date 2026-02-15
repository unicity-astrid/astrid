//! Edit file tool — performs exact string replacements in files.

use crate::{BuiltinTool, ToolContext, ToolError, ToolResult};
use serde_json::Value;

/// Built-in tool for editing files via string replacement.
pub struct EditFileTool;

#[async_trait::async_trait]
impl BuiltinTool for EditFileTool {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn description(&self) -> &'static str {
        "Performs exact string replacements in files. The old_string must be unique in the file \
         unless replace_all is true. Fails if old_string is not found or matches multiple times \
         (without replace_all)."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default: false)",
                    "default": false
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let file_path = args
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArguments("file_path is required".into()))?;

        let old_string = args
            .get("old_string")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArguments("old_string is required".into()))?;

        let new_string = args
            .get("new_string")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArguments("new_string is required".into()))?;

        let replace_all = args
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let path = std::path::Path::new(file_path);
        if !path.exists() {
            return Err(ToolError::PathNotFound(file_path.to_string()));
        }

        let content = tokio::fs::read_to_string(path).await?;

        // Count occurrences
        let count = content.matches(old_string).count();

        if count == 0 {
            return Err(ToolError::ExecutionFailed(format!(
                "old_string not found in {file_path}"
            )));
        }

        if count > 1 && !replace_all {
            return Err(ToolError::ExecutionFailed(format!(
                "old_string found {count} times in {file_path} — use replace_all or provide more context to make it unique"
            )));
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        tokio::fs::write(path, &new_content).await?;

        if replace_all && count > 1 {
            Ok(format!("Replaced {count} occurrences in {file_path}"))
        } else {
            Ok(format!("Edited {file_path}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn ctx() -> ToolContext {
        ToolContext::new(std::env::temp_dir())
    }

    #[tokio::test]
    async fn test_edit_file_basic() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "hello world").unwrap();

        let result = EditFileTool
            .execute(
                serde_json::json!({
                    "file_path": f.path().to_str().unwrap(),
                    "old_string": "hello",
                    "new_string": "goodbye"
                }),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(result.contains("Edited"));
        assert_eq!(std::fs::read_to_string(f.path()).unwrap(), "goodbye world");
    }

    #[tokio::test]
    async fn test_edit_file_not_found() {
        let result = EditFileTool
            .execute(
                serde_json::json!({
                    "file_path": "/tmp/astrid_nonexistent_12345.txt",
                    "old_string": "a",
                    "new_string": "b"
                }),
                &ctx(),
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_edit_file_old_string_not_found() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "hello world").unwrap();

        let result = EditFileTool
            .execute(
                serde_json::json!({
                    "file_path": f.path().to_str().unwrap(),
                    "old_string": "foobar",
                    "new_string": "baz"
                }),
                &ctx(),
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_edit_file_non_unique_fails() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "aaa bbb aaa").unwrap();

        let result = EditFileTool
            .execute(
                serde_json::json!({
                    "file_path": f.path().to_str().unwrap(),
                    "old_string": "aaa",
                    "new_string": "ccc"
                }),
                &ctx(),
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("2 times"));
    }

    #[tokio::test]
    async fn test_edit_file_replace_all() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "aaa bbb aaa").unwrap();

        let result = EditFileTool
            .execute(
                serde_json::json!({
                    "file_path": f.path().to_str().unwrap(),
                    "old_string": "aaa",
                    "new_string": "ccc",
                    "replace_all": true
                }),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(result.contains("2 occurrences"));
        assert_eq!(std::fs::read_to_string(f.path()).unwrap(), "ccc bbb ccc");
    }
}
