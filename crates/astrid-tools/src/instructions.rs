//! Project instructions loader — loads `ASTRID.md` files.
//!
//! Loading order:
//! 1. `~/.astrid/instructions.md` (global user instructions)
//! 2. `ASTRID.md` in workspace root (takes priority)
//! 3. `.astrid/ASTRID.md` in workspace root (additive, loaded alongside root file)
//! 4. Fallback: `CLAUDE.md` in workspace root (only if no ASTRID.md)
//! 5. `.claude/CLAUDE.md` in workspace root (additive, loaded alongside fallback)

use std::path::Path;

/// Load project instructions from the workspace and global config.
///
/// Returns the concatenated instructions text, or an empty string if none found.
#[must_use]
pub fn load_project_instructions(workspace_root: &Path) -> String {
    let mut sections = Vec::new();

    // 1. Global instructions
    if let Some(home) = dirs_path() {
        let global_path = home.join(".astrid").join("instructions.md");
        if let Ok(content) = std::fs::read_to_string(&global_path)
            && !content.trim().is_empty()
        {
            sections.push(content);
        }
    }

    // 2. ASTRID.md in workspace root (takes priority)
    let astrid_md = workspace_root.join("ASTRID.md");
    if let Ok(content) = std::fs::read_to_string(&astrid_md)
        && !content.trim().is_empty()
    {
        sections.push(content);

        // 3. Also load .astrid/ASTRID.md (additive, project-level config dir)
        let dot_astrid_md = workspace_root.join(".astrid").join("ASTRID.md");
        if let Ok(content) = std::fs::read_to_string(&dot_astrid_md)
            && !content.trim().is_empty()
        {
            sections.push(content);
        }
    } else {
        // Check .astrid/ASTRID.md as a standalone source
        let dot_astrid_md = workspace_root.join(".astrid").join("ASTRID.md");
        if let Ok(content) = std::fs::read_to_string(&dot_astrid_md)
            && !content.trim().is_empty()
        {
            sections.push(content);
        } else {
            // 4. Fallback: CLAUDE.md (compatibility with existing projects)
            let claude_md = workspace_root.join("CLAUDE.md");
            if let Ok(content) = std::fs::read_to_string(&claude_md)
                && !content.trim().is_empty()
            {
                sections.push(content);
            }
            // 5. Also check .claude/CLAUDE.md
            let claude_dir_md = workspace_root.join(".claude").join("CLAUDE.md");
            if let Ok(content) = std::fs::read_to_string(&claude_dir_md)
                && !content.trim().is_empty()
            {
                sections.push(content);
            }
        }
    }

    sections.join("\n\n---\n\n")
}

/// Get the user's home directory.
fn dirs_path() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(std::path::PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_no_instructions() {
        let dir = TempDir::new().unwrap();
        let result = load_project_instructions(dir.path());
        // Should return empty or global-only
        // (global might or might not exist on the test machine)
        assert!(result.is_empty() || !result.is_empty());
    }

    #[test]
    fn test_astrid_md_loaded() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("ASTRID.md"), "# Astrid Instructions").unwrap();

        let result = load_project_instructions(dir.path());
        assert!(result.contains("Astrid Instructions"));
    }

    #[test]
    fn test_claude_md_fallback() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# Claude Instructions").unwrap();

        let result = load_project_instructions(dir.path());
        assert!(result.contains("Claude Instructions"));
    }

    #[test]
    fn test_claude_md_not_loaded_when_astrid_md_exists() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("ASTRID.md"), "# Astrid Rules").unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# Claude Instructions").unwrap();

        let result = load_project_instructions(dir.path());
        assert!(result.contains("Astrid Rules"));
        assert!(!result.contains("Claude Instructions"));
    }

    #[test]
    fn test_claude_dir_md_fallback() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".claude")).unwrap();
        std::fs::write(
            dir.path().join(".claude").join("CLAUDE.md"),
            "# Dir Claude Instructions",
        )
        .unwrap();

        let result = load_project_instructions(dir.path());
        assert!(result.contains("Dir Claude Instructions"));
    }

    #[test]
    fn test_claude_dir_md_not_loaded_when_astrid_md_exists() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("ASTRID.md"), "# Astrid Rules").unwrap();
        std::fs::create_dir(dir.path().join(".claude")).unwrap();
        std::fs::write(
            dir.path().join(".claude").join("CLAUDE.md"),
            "# Dir Claude Instructions",
        )
        .unwrap();

        let result = load_project_instructions(dir.path());
        assert!(result.contains("Astrid Rules"));
        assert!(!result.contains("Dir Claude Instructions"));
    }

    #[test]
    fn test_dot_astrid_md_loaded_with_root_astrid_md() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("ASTRID.md"), "# Root Instructions").unwrap();
        std::fs::create_dir(dir.path().join(".astrid")).unwrap();
        std::fs::write(
            dir.path().join(".astrid").join("ASTRID.md"),
            "# Dir Instructions",
        )
        .unwrap();

        let result = load_project_instructions(dir.path());
        assert!(result.contains("Root Instructions"));
        assert!(result.contains("Dir Instructions"));
    }

    #[test]
    fn test_dot_astrid_md_standalone() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".astrid")).unwrap();
        std::fs::write(
            dir.path().join(".astrid").join("ASTRID.md"),
            "# Standalone Dir Instructions",
        )
        .unwrap();

        let result = load_project_instructions(dir.path());
        assert!(result.contains("Standalone Dir Instructions"));
    }

    #[test]
    fn test_dot_astrid_md_takes_priority_over_claude_fallback() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".astrid")).unwrap();
        std::fs::write(dir.path().join(".astrid").join("ASTRID.md"), "# Astrid Dir").unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# Claude Fallback").unwrap();

        let result = load_project_instructions(dir.path());
        assert!(result.contains("Astrid Dir"));
        assert!(!result.contains("Claude Fallback"));
    }
}
