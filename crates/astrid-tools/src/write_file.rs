//! Write file tool — writes content to a file, creating parent directories as needed.

use crate::{BuiltinTool, ToolContext, ToolError, ToolResult};
use serde_json::Value;

/// Built-in tool for writing files.
pub struct WriteFileTool;

#[async_trait::async_trait]
impl BuiltinTool for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "Writes content to a file. Creates parent directories if they don't exist. \
         Overwrites the file if it already exists."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let file_path = args
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArguments("file_path is required".into()))?;

        let content = args
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArguments("content is required".into()))?;

        let path = std::path::Path::new(file_path);
        if !path.is_absolute() {
            return Err(ToolError::InvalidArguments(
                "file_path must be an absolute path".into(),
            ));
        }

        // Create parent directories
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(path, content).await?;

        let bytes = content.len();
        Ok(format!("Wrote {bytes} bytes to {file_path}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ctx() -> ToolContext {
        ToolContext::new(std::env::temp_dir(), None)
    }

    #[tokio::test]
    async fn test_write_file_basic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");

        let result = WriteFileTool
            .execute(
                serde_json::json!({
                    "file_path": path.to_str().unwrap(),
                    "content": "hello world"
                }),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(result.contains("11 bytes"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[tokio::test]
    async fn test_write_file_creates_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a").join("b").join("c").join("test.txt");

        WriteFileTool
            .execute(
                serde_json::json!({
                    "file_path": path.to_str().unwrap(),
                    "content": "nested"
                }),
                &ctx(),
            )
            .await
            .unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "nested");
    }

    #[tokio::test]
    async fn test_write_file_overwrites() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "old content").unwrap();

        WriteFileTool
            .execute(
                serde_json::json!({
                    "file_path": path.to_str().unwrap(),
                    "content": "new content"
                }),
                &ctx(),
            )
            .await
            .unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new content");
    }

    #[tokio::test]
    async fn test_write_file_missing_args() {
        let result = WriteFileTool
            .execute(serde_json::json!({"file_path": "/tmp/test.txt"}), &ctx())
            .await;
        assert!(result.is_err());

        let result = WriteFileTool
            .execute(serde_json::json!({"content": "hello"}), &ctx())
            .await;
        assert!(result.is_err());
    }
}
