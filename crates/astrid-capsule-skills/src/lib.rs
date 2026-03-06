#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

use astrid_sdk::prelude::*;
use astrid_sdk::schemars;
use serde::{Deserialize, Serialize};

#[derive(Default)]
pub struct SkillsLoader;

#[derive(Debug, Deserialize)]
struct VfsDirEntry {
    name: String,
    is_dir: bool,
}

#[derive(Debug, PartialEq)]
struct SkillFrontmatter {
    name: String,
    description: String,
}

#[derive(Debug, Serialize)]
struct SkillInfo {
    id: String,
    name: String,
    description: String,
}

#[derive(Debug, Default, Deserialize, astrid_sdk::schemars::JsonSchema)]
pub struct ListSkillsArgs {
    /// Directory containing the skills (e.g., ".gemini/skills")
    pub dir_path: String,
}

#[derive(Debug, Default, Deserialize, astrid_sdk::schemars::JsonSchema)]
pub struct ReadSkillArgs {
    /// Directory containing the skills (e.g., ".gemini/skills")
    pub dir_path: String,
    /// The ID/folder name of the skill to read
    pub skill_id: String,
}

#[capsule]
impl SkillsLoader {
    #[astrid::tool("list_skills")]
    pub fn list_skills(&self, args: ListSkillsArgs) -> Result<String, SysError> {
        if args.dir_path.contains("..") || args.dir_path.contains('\0') {
            return Err(SysError::ApiError(
                "Invalid dir_path: path traversal detected".into(),
            ));
        }
        let clean_dir = args.dir_path.trim_end_matches('/');

        let bytes = match fs::readdir(clean_dir) {
            Ok(b) => b,
            Err(e) => {
                let err_str = e.to_string().to_lowercase();
                if err_str.contains("not found") || err_str.contains("no such file") {
                    return Ok("[]".to_string());
                }
                return Err(e);
            },
        };

        let entries: Vec<VfsDirEntry> = serde_json::from_slice(&bytes)
            .map_err(|e| SysError::ApiError(format!("Failed to parse dir entries: {e}")))?;

        let mut skills = Vec::new();

        for entry in entries {
            if !entry.is_dir || !is_safe_name(&entry.name) {
                continue;
            }
            let skill_path = format!("{}/{}/SKILL.md", clean_dir, entry.name);
            if let Ok(content) = fs::read_string(&skill_path) {
                if let Some(fm) = parse_frontmatter(&content) {
                    skills.push(SkillInfo {
                        id: entry.name.clone(),
                        name: fm.name,
                        description: fm.description,
                    });
                } else {
                    let _ = sys::log(
                        "warn",
                        format!("skipping {}: invalid frontmatter", entry.name),
                    );
                }
            } else {
                let _ = sys::log(
                    "debug",
                    format!("skipping {}: no SKILL.md found", entry.name),
                );
            }
        }

        let json = serde_json::to_string(&skills)?;
        Ok(json)
    }

    #[astrid::tool("read_skill")]
    pub fn read_skill(&self, args: ReadSkillArgs) -> Result<String, SysError> {
        let skill_path = resolve_skill_path(&args.dir_path, &args.skill_id)?;
        match fs::read_string(&skill_path) {
            Ok(content) => Ok(content),
            Err(_) => Err(SysError::ApiError(format!(
                "Skill '{}' not found",
                args.skill_id
            ))),
        }
    }
}

/// Returns true if `name` is a safe single path component (no traversal).
fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && !name.contains("..")
}

fn resolve_skill_path(dir_path: &str, skill_id: &str) -> Result<String, SysError> {
    if dir_path.contains("..") || dir_path.contains('\0') {
        return Err(SysError::ApiError(
            "Invalid dir_path: path traversal detected".into(),
        ));
    }
    let clean_dir = dir_path.trim_end_matches('/');

    if !is_safe_name(skill_id) {
        return Err(SysError::ApiError(
            "Invalid skill_id: path traversal detected".into(),
        ));
    }

    Ok(format!("{}/{}/SKILL.md", clean_dir, skill_id))
}

/// Parse YAML frontmatter from a SKILL.md file.
///
/// Extracts `name` and `description` fields from the `---` delimited header.
/// Uses manual key: value parsing to avoid pulling in a YAML library for
/// two trivial fields — and to guarantee WASM compatibility.
fn parse_frontmatter(content: &str) -> Option<SkillFrontmatter> {
    // Skip the opening delimiter and any trailing whitespace on that line
    let rest = content.strip_prefix("---")?;
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))?;

    // Find the closing delimiter — `---` must be on its own line
    let end_idx = rest
        .match_indices("\n---")
        .find(|&(idx, _)| {
            let after = idx + 4; // "\n---".len()
            matches!(rest.as_bytes().get(after), None | Some(b'\n') | Some(b'\r'))
        })
        .map(|(idx, _)| idx)?;
    let block = &rest[..end_idx];

    let mut name = None;
    let mut description = None;

    for line in block.lines() {
        let line = line.trim();
        if let Some((key, val)) = line.split_once(':') {
            let key = key.trim();
            let val = val.trim();
            match key {
                "name" => name = Some(val.to_string()),
                "description" => description = Some(val.to_string()),
                _ => {},
            }
        }
    }

    Some(SkillFrontmatter {
        name: name?,
        description: description?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_frontmatter() {
        let content =
            "---\nname: my-skill\ndescription: Does a thing\n---\n# My Skill\nSome content";
        let parsed = parse_frontmatter(content).unwrap();
        assert_eq!(parsed.name, "my-skill");
        assert_eq!(parsed.description, "Does a thing");
    }

    #[test]
    fn test_parse_stops_at_first_closing_delimiter() {
        let content =
            "---\nname: test\ndescription: testing\n---\n# Test\n---\nSome text\n---\nMore text";
        let parsed = parse_frontmatter(content).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.description, "testing");
    }

    #[test]
    fn test_parse_no_frontmatter() {
        let content = "# Title\nJust some text";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_unclosed_frontmatter() {
        let content = "---\nname: test\ndescription: missing end rule\n# Oops";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_frontmatter_crlf() {
        let content =
            "---\r\nname: crlf-skill\r\ndescription: Windows line endings\r\n---\r\n# Content";
        let parsed = parse_frontmatter(content).unwrap();
        assert_eq!(parsed.name, "crlf-skill");
        assert_eq!(parsed.description, "Windows line endings");
    }

    #[test]
    fn test_parse_frontmatter_missing_field() {
        let content = "---\nname: only-name\n---\n# Content";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_frontmatter_delimiter_must_be_own_line() {
        // "---notadash" should not be treated as a closing delimiter
        let content = "---\nname: test\n---notadash\ndescription: real desc\n---\n# Body";
        let parsed = parse_frontmatter(content).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.description, "real desc");
    }

    #[test]
    fn test_is_safe_name() {
        assert!(is_safe_name("valid-skill"));
        assert!(is_safe_name("skill_v2"));
        assert!(!is_safe_name(""));
        assert!(!is_safe_name("../escape"));
        assert!(!is_safe_name("some/path"));
        assert!(!is_safe_name("back\\slash"));
        assert!(!is_safe_name(".."));
        assert!(!is_safe_name("."));
        assert!(!is_safe_name("skill\0null"));
        assert!(!is_safe_name("skill\0"));
    }

    #[test]
    fn test_path_traversal_null_bytes() {
        assert!(resolve_skill_path("skills\0evil", "ok").is_err());
        assert!(resolve_skill_path("skills", "ok\0evil").is_err());
    }

    #[test]
    fn test_parse_frontmatter_description_with_colon() {
        let content = "---\nname: deploy\ndescription: Runs the deploy: prod pipeline\n---\n# Body";
        let parsed = parse_frontmatter(content).unwrap();
        assert_eq!(parsed.description, "Runs the deploy: prod pipeline");
    }

    #[test]
    fn test_path_traversal_check() {
        assert!(resolve_skill_path(".gemini/skills", "../secret/file").is_err());
        assert!(resolve_skill_path(".gemini/skills", "some/folder").is_err());
        assert!(resolve_skill_path(".gemini/skills", "..\\windows\\hack").is_err());
        assert!(resolve_skill_path("../escaped", "skill").is_err());

        let valid = resolve_skill_path(".gemini/skills/", "valid-skill-id").unwrap();
        assert_eq!(valid, ".gemini/skills/valid-skill-id/SKILL.md");

        let valid2 = resolve_skill_path("skills", "skill_version_2").unwrap();
        assert_eq!(valid2, "skills/skill_version_2/SKILL.md");
    }
}
