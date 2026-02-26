#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![allow(missing_docs)]

//! Filesystem tools capsule for Astrid OS.
//!
//! Provides `read_file`, `write_file`, `replace_in_file`, `list_directory`,
//! and `grep_search` tools to agents.

use astrid_sdk::prelude::*;
use serde::Deserialize;

#[derive(Default)]
pub struct FsTools;

#[derive(Debug, Default, Deserialize, astrid_sdk::schemars::JsonSchema)]
pub struct ReadFileArgs {
    pub file_path: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
}

#[derive(Debug, Default, Deserialize, astrid_sdk::schemars::JsonSchema)]
pub struct WriteFileArgs {
    pub file_path: String,
    pub content: String,
}

#[derive(Debug, Default, Deserialize, astrid_sdk::schemars::JsonSchema)]
pub struct ReplaceInFileArgs {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
}

#[derive(Debug, Default, Deserialize, astrid_sdk::schemars::JsonSchema)]
pub struct ListDirectoryArgs {
    pub dir_path: String,
}

#[derive(Debug, Default, Deserialize, astrid_sdk::schemars::JsonSchema)]
pub struct GrepSearchArgs {
    pub dir_path: Option<String>,
    pub pattern: String,
}

#[capsule]
impl FsTools {
    #[astrid::tool("read_file")]
    pub fn read_file(&self, args: ReadFileArgs) -> Result<String, SysError> {
        // Use the VFS Airlock to read the file
        // Note: SDK does not currently have read_string with lines, we can just use read_string and parse lines manually for now.
        let content = fs::read_string(&args.file_path)?;

        let lines: Vec<&str> = content.lines().collect();
        let start = args.start_line.unwrap_or(1).saturating_sub(1);
        let end = args.end_line.unwrap_or(lines.len()).min(lines.len());

        if start >= lines.len() || start >= end {
            return Ok(String::new());
        }

        let slice = &lines[start..end];
        Ok(slice.join("\n"))
    }

    #[astrid::tool("write_file")]
    pub fn write_file(&self, args: WriteFileArgs) -> Result<String, SysError> {
        fs::write_string(&args.file_path, &args.content)?;
        Ok(format!("Successfully wrote to {}", args.file_path))
    }

    #[astrid::tool("replace_in_file")]
    pub fn replace_in_file(&self, args: ReplaceInFileArgs) -> Result<String, SysError> {
        let content = fs::read_string(&args.file_path)?;
        
        let count = content.matches(&args.old_string).count();
        if count == 0 {
            return Err(SysError::ApiError(format!("Exact string not found in {}", args.file_path)));
        }
        if count > 1 {
            return Err(SysError::ApiError(format!("Found {} occurrences of string in {}. Please be more specific.", count, args.file_path)));
        }

        let new_content = content.replace(&args.old_string, &args.new_string);
        fs::write_string(&args.file_path, &new_content)?;

        Ok(format!("Successfully replaced text in {}", args.file_path))
    }

    #[astrid::tool("list_directory")]
    pub fn list_directory(&self, args: ListDirectoryArgs) -> Result<String, SysError> {
        let bytes = fs::readdir(&args.dir_path)?;
        // Currently assuming it returns JSON array of entries. Let's just return raw string for now
        // if we haven't typed it in SDK.
        let result = String::from_utf8(bytes).map_err(|e| SysError::ApiError(e.to_string()))?;
        Ok(result)
    }

    #[astrid::tool("grep_search")]
    pub fn grep_search(&self, _args: GrepSearchArgs) -> Result<String, SysError> {
        // Stub implementation, full grep would require recursively iterating directories
        // and applying regex, but we will rely on the host system or a rust regex loop.
        // For simplicity right now, returning error until full recursive ripgrep logic is added.
        Err(SysError::ApiError("grep_search is not yet implemented in fs-tools capsule".into()))
    }
}
