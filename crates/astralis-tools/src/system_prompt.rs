//! System prompt assembly — builds the full system prompt with tool guidelines.

use std::path::Path;

use crate::instructions::load_project_instructions;

/// Build the complete system prompt for an agent session.
///
/// Assembles: base prompt + workspace context + tool guidelines + project instructions.
#[must_use]
pub fn build_system_prompt(workspace_root: &Path) -> String {
    let project_name = workspace_root.file_name().map_or_else(
        || "project".to_string(),
        |n| n.to_string_lossy().to_string(),
    );

    let instructions = load_project_instructions(workspace_root);

    let os = std::env::consts::OS;

    let mut prompt = format!(
        "You are an AI coding assistant working in the project \"{project_name}\".\n\n\
         # Environment\n\
         - Current working directory: {workspace}\n\
         - Platform: {os}\n\n",
        workspace = workspace_root.display()
    );

    // Tool usage guidelines
    prompt.push_str(TOOL_GUIDELINES);

    // Project instructions
    if !instructions.is_empty() {
        prompt.push_str("\n\n# Project Instructions\n\n");
        prompt.push_str(&instructions);
    }

    prompt
}

/// Tool usage guidelines for the LLM.
const TOOL_GUIDELINES: &str = "\
# Tool Usage Guidelines

## File Operations
- Always read a file before editing it — understand existing code before modifying.
- Prefer `edit_file` over `write_file` for existing files — edits are safer and more precise.
- Use `read_file` with offset/limit for large files instead of reading the entire file.

## Search
- Use `glob` to find files by name pattern before using `grep` to search contents.
- Use `grep` with a file glob filter to narrow searches to relevant file types.

## Execution
- Use `bash` for git, build tools, package managers, and other terminal operations.
- Do NOT use `bash` for file operations — use the dedicated file tools instead.
- The bash working directory persists between calls.

## General
- Read before writing. Understand before changing.
- Make minimal, focused changes. Don't add unnecessary modifications.";

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_build_system_prompt_basic() {
        let dir = TempDir::new().unwrap();
        let prompt = build_system_prompt(dir.path());

        assert!(prompt.contains("AI coding assistant"));
        assert!(prompt.contains("Tool Usage Guidelines"));
        assert!(prompt.contains("File Operations"));
    }

    #[test]
    fn test_build_system_prompt_with_instructions() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("ASTRALIS.md"),
            "# Custom Rules\nDo X not Y.",
        )
        .unwrap();

        let prompt = build_system_prompt(dir.path());

        assert!(prompt.contains("Project Instructions"));
        assert!(prompt.contains("Custom Rules"));
        assert!(prompt.contains("Do X not Y"));
    }

    #[test]
    fn test_build_system_prompt_includes_workspace_path() {
        let dir = TempDir::new().unwrap();
        let prompt = build_system_prompt(dir.path());

        assert!(prompt.contains(&dir.path().display().to_string()));
    }
}
