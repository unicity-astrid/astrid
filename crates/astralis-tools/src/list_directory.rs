//! List directory tool — lists directory contents with type and size info.

use std::fmt::Write;

use crate::{BuiltinTool, ToolContext, ToolError, ToolResult};
use serde_json::Value;
use std::path::PathBuf;

/// Built-in tool for listing directory contents.
pub struct ListDirectoryTool;

#[async_trait::async_trait]
impl BuiltinTool for ListDirectoryTool {
    fn name(&self) -> &'static str {
        "list_directory"
    }

    fn description(&self) -> &'static str {
        "Lists the contents of a directory. Shows directories first, then files, \
         both sorted alphabetically. Includes type indicator and file sizes."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the directory to list"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let dir_path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArguments("path is required".into()))?;

        let path = PathBuf::from(dir_path);
        if !path.exists() {
            return Err(ToolError::PathNotFound(dir_path.to_string()));
        }
        if !path.is_dir() {
            return Err(ToolError::InvalidArguments(format!(
                "{dir_path} is not a directory"
            )));
        }

        let mut dirs: Vec<String> = Vec::new();
        let mut files: Vec<String> = Vec::new();

        let mut entries = tokio::fs::read_dir(&path).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            let metadata = entry.metadata().await?;

            if metadata.is_dir() {
                dirs.push(format!("  {name}/"));
            } else {
                let size = metadata.len();
                let size_str = format_size(size);
                files.push(format!("  {name}  ({size_str})"));
            }
        }

        dirs.sort();
        files.sort();

        let mut output = String::new();
        for d in &dirs {
            output.push_str(d);
            output.push('\n');
        }
        for f in &files {
            output.push_str(f);
            output.push('\n');
        }

        let total = dirs.len().saturating_add(files.len());

        let _ = write!(
            output,
            "\n({} directories, {} files)",
            dirs.len(),
            files.len()
        );

        if total == 0 {
            return Ok(format!("{dir_path} is empty"));
        }

        Ok(output)
    }
}

/// Format a byte count into a human-readable size string.
#[allow(clippy::cast_precision_loss)]
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ctx() -> ToolContext {
        ToolContext::new(std::env::temp_dir())
    }

    #[tokio::test]
    async fn test_list_directory_basic() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        std::fs::write(dir.path().join("file.txt"), "hello").unwrap();

        let result = ListDirectoryTool
            .execute(
                serde_json::json!({"path": dir.path().to_str().unwrap()}),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(result.contains("subdir/"));
        assert!(result.contains("file.txt"));
        assert!(result.contains("1 directories, 1 files"));
    }

    #[tokio::test]
    async fn test_list_directory_dirs_first() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("aaa.txt"), "").unwrap();
        std::fs::create_dir(dir.path().join("zzz")).unwrap();

        let result = ListDirectoryTool
            .execute(
                serde_json::json!({"path": dir.path().to_str().unwrap()}),
                &ctx(),
            )
            .await
            .unwrap();

        let dir_pos = result.find("zzz/").unwrap();
        let file_pos = result.find("aaa.txt").unwrap();
        assert!(dir_pos < file_pos);
    }

    #[tokio::test]
    async fn test_list_directory_not_found() {
        let result = ListDirectoryTool
            .execute(
                serde_json::json!({"path": "/tmp/astralis_nonexistent_dir_12345"}),
                &ctx(),
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_directory_not_a_dir() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("file.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let result = ListDirectoryTool
            .execute(
                serde_json::json!({"path": file_path.to_str().unwrap()}),
                &ctx(),
            )
            .await;

        assert!(result.is_err());
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1_048_576), "1.0 MB");
        assert_eq!(format_size(1_073_741_824), "1.0 GB");
    }
}
