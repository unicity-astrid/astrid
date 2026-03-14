#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![allow(missing_docs)]

//! Filesystem tools capsule for Astrid OS.
//!
//! Provides `read_file`, `write_file`, `replace_in_file`, `list_directory`,
//! `grep_search`, `create_directory`, `delete_file`, and `move_file` tools to agents.

mod grep;

use astrid_sdk::prelude::*;
use astrid_sdk::schemars;
use grep::{GREP_MAX_DEPTH, GREP_MAX_FILES, GREP_MAX_MATCHES, grep_content};
use serde::Deserialize;

/// Maximum file size (10 MB) that `move_file` will transit through WASM guest memory.
const MOVE_FILE_MAX_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Default)]
pub struct FsTools;

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct ReadFileArgs {
    pub file_path: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct WriteFileArgs {
    pub file_path: String,
    pub content: String,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct ReplaceInFileArgs {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct ListDirectoryArgs {
    pub dir_path: String,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct GrepSearchArgs {
    pub dir_path: Option<String>,
    pub pattern: String,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct CreateDirectoryArgs {
    pub dir_path: String,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct DeleteFileArgs {
    pub file_path: String,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct MoveFileArgs {
    pub source_path: String,
    pub destination_path: String,
}

#[capsule]
impl FsTools {
    #[astrid::tool("read_file")]
    pub fn read_file(&self, args: ReadFileArgs) -> Result<String, SysError> {
        // Use the VFS Airlock to read the file
        // Note: SDK does not currently have read_to_string with lines, we can just use read_to_string and parse lines manually for now.
        let content = fs::read_to_string(&args.file_path)?;

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
        fs::write(&args.file_path, &args.content)?;
        Ok(format!("Successfully wrote to {}", args.file_path))
    }

    #[astrid::tool("replace_in_file")]
    pub fn replace_in_file(&self, args: ReplaceInFileArgs) -> Result<String, SysError> {
        let content = fs::read_to_string(&args.file_path)?;

        let count = content.matches(&args.old_string).count();
        if count == 0 {
            return Err(SysError::ApiError(format!("Exact string not found in {}", args.file_path)));
        }
        if count > 1 {
            return Err(SysError::ApiError(format!("Found {} occurrences of string in {}. Please be more specific.", count, args.file_path)));
        }

        let new_content = content.replace(&args.old_string, &args.new_string);
        fs::write(&args.file_path, &new_content)?;

        Ok(format!("Successfully replaced text in {}", args.file_path))
    }

    #[astrid::tool("list_directory")]
    pub fn list_directory(&self, args: ListDirectoryArgs) -> Result<String, SysError> {
        let bytes = fs::read_dir(&args.dir_path)?;
        // Currently assuming it returns JSON array of entries. Let's just return raw string for now
        // if we haven't typed it in SDK.
        let result = String::from_utf8(bytes).map_err(|e| SysError::ApiError(e.to_string()))?;
        Ok(result)
    }

    #[astrid::tool("grep_search")]
    pub fn grep_search(&self, args: GrepSearchArgs) -> Result<String, SysError> {
        if args.pattern.is_empty() {
            return Err(SysError::ApiError("pattern must not be empty".into()));
        }

        let root = args.dir_path.as_deref().unwrap_or(".");
        let mut matches: Vec<String> = Vec::new();
        let mut files_visited: usize = 0;

        walk_and_grep(
            root,
            &args.pattern,
            &mut matches,
            &mut files_visited,
            0,
        );

        if matches.is_empty() {
            return Ok("No matches found.".into());
        }

        Ok(matches.join("\n"))
    }

    #[astrid::tool("create_directory")]
    pub fn create_directory(&self, args: CreateDirectoryArgs) -> Result<String, SysError> {
        fs::create_dir(&args.dir_path)?;
        Ok(format!("Successfully created directory {}", args.dir_path))
    }

    #[astrid::tool("delete_file")]
    pub fn delete_file(&self, args: DeleteFileArgs) -> Result<String, SysError> {
        if !fs::exists(&args.file_path)? {
            return Err(SysError::ApiError(format!(
                "file does not exist: {}",
                args.file_path
            )));
        }
        if file_stat(&args.file_path)?.is_dir {
            return Err(SysError::ApiError(format!(
                "{} is a directory, not a file; delete_file only supports files",
                args.file_path
            )));
        }
        fs::remove_file(&args.file_path)?;
        Ok(format!("Successfully deleted {}", args.file_path))
    }

    #[astrid::tool("move_file")]
    pub fn move_file(&self, args: MoveFileArgs) -> Result<String, SysError> {
        // Single stat covers both existence and directory checks.
        let src_stat = match file_stat(&args.source_path) {
            Ok(s) => s,
            Err(_) => {
                return Err(SysError::ApiError(format!(
                    "source path does not exist: {}",
                    args.source_path
                )));
            }
        };
        if src_stat.is_dir {
            return Err(SysError::ApiError(format!(
                "{} is a directory, not a file; move_file only supports files",
                args.source_path
            )));
        }
        if src_stat.size > MOVE_FILE_MAX_BYTES {
            return Err(SysError::ApiError(format!(
                "source file is too large to move ({} bytes, limit is {} bytes)",
                src_stat.size, MOVE_FILE_MAX_BYTES
            )));
        }
        if fs::exists(&args.destination_path)? {
            return Err(SysError::ApiError(format!(
                "destination already exists: {}",
                args.destination_path
            )));
        }

        let content = fs::read(&args.source_path)?;
        fs::write(&args.destination_path, &content)?;

        if let Err(e) = fs::remove_file(&args.source_path) {
            // Destination was written; clean up to avoid a phantom copy.
            let _ = fs::remove_file(&args.destination_path);
            return Err(SysError::ApiError(format!(
                "move failed: source could not be removed ({e}); destination write was rolled back"
            )));
        }

        Ok(format!(
            "Successfully moved {} to {}",
            args.source_path, args.destination_path
        ))
    }
}

/// Parsed VFS metadata for a single path.
struct FileStat {
    is_dir: bool,
    size: u64,
}

/// Returns parsed metadata for `path`, or a clear "not found" error.
fn file_stat(path: &str) -> Result<FileStat, SysError> {
    let stat_bytes = fs::metadata(path)?;
    let val: serde_json::Value = serde_json::from_slice(&stat_bytes)
        .map_err(|e| SysError::ApiError(format!("failed to parse metadata for {path}: {e}")))?;
    let is_dir = val.get("isDir").and_then(|v| v.as_bool()).unwrap_or(false);
    let size = val.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
    Ok(FileStat { is_dir, size })
}

/// Recursively walks `dir` and collects lines containing `pattern`.
///
/// Respects depth, file-count, and match-count caps to prevent runaway searches.
fn walk_and_grep(
    dir: &str,
    pattern: &str,
    matches: &mut Vec<String>,
    files_visited: &mut usize,
    depth: usize,
) {
    if depth >= GREP_MAX_DEPTH
        || *files_visited >= GREP_MAX_FILES
        || matches.len() >= GREP_MAX_MATCHES
    {
        return;
    }

    let entries_bytes = match fs::read_dir(dir) {
        Ok(b) => b,
        Err(e) => {
            let _ = log::debug(&format!("failed to read directory '{dir}': {e}"));
            return;
        }
    };

    let entry_names: Vec<String> = match serde_json::from_slice(&entries_bytes) {
        Ok(v) => v,
        Err(e) => {
            let _ = log::warn(&format!("failed to parse directory entries for '{dir}': {e}"));
            return;
        }
    };

    for name in entry_names {
        if matches.len() >= GREP_MAX_MATCHES || *files_visited >= GREP_MAX_FILES {
            return;
        }

        let path = std::path::PathBuf::from(dir)
            .join(&name)
            .to_string_lossy()
            .into_owned();

        let stat_bytes = match fs::metadata(&path) {
            Ok(b) => b,
            Err(e) => {
                let _ = log::debug(&format!("failed to stat path '{path}': {e}"));
                continue;
            }
        };

        let is_dir = serde_json::from_slice::<serde_json::Value>(&stat_bytes)
            .ok()
            .and_then(|v| v.get("isDir")?.as_bool())
            .unwrap_or(false);

        if is_dir {
            walk_and_grep(&path, pattern, matches, files_visited, depth + 1);
        } else {
            *files_visited += 1;
            grep_file(&path, pattern, matches);
        }
    }
}

/// Searches a single file for lines containing `pattern`, appending
/// `path:line_number:content` to `matches`.
fn grep_file(path: &str, pattern: &str, matches: &mut Vec<String>) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            let _ = log::debug(&format!("skipping unreadable file '{path}': {e}"));
            return;
        }
    };

    grep_content(path, &content, pattern, matches);
}
