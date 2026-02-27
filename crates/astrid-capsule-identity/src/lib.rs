#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![warn(missing_docs)]

//! Identity builder capsule for Astrid OS.
//!
//! Subscribes to `identity.request.build` IPC events and generates the
//! system prompt for the orchestrator capsule by reading workspace
//! configuration files (AGENTS.md, .astridignore) and the spark identity.

use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

/// Identity builder capsule. Stateless — reads workspace files on each request.
#[derive(Default)]
pub struct IdentityBuilder;

/// Request payload for building the system prompt.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct BuildRequest {
    /// Absolute path to the workspace root directory.
    pub workspace_root: String,
    /// Optional agent identity configuration (from spark.toml).
    pub spark: Option<SparkConfig>,
}

/// Agent identity configuration fields from spark.toml.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SparkConfig {
    /// Agent name/identifier.
    pub callsign: String,
    /// Agent role description.
    pub class: String,
    /// Personality traits.
    pub aura: String,
    /// Communication style preferences.
    pub signal: String,
    /// Core directives and constraints.
    pub core: String,
}

impl SparkConfig {
    /// Build the identity preamble from spark fields.
    /// Returns `None` if the callsign is empty.
    fn build_preamble(&self) -> Option<String> {
        if self.callsign.is_empty() {
            return None;
        }

        let mut parts = vec![];
        if !self.class.is_empty() {
            parts.push(format!("You are {}, a {}.", self.callsign, self.class));
        } else {
            parts.push(format!("You are {}.", self.callsign));
        }

        if !self.aura.is_empty() {
            parts.push(format!("# Personality\n{}", self.aura));
        }
        if !self.signal.is_empty() {
            parts.push(format!("# Communication Style\n{}", self.signal));
        }
        if !self.core.is_empty() {
            parts.push(format!("# Core Directives\n{}", self.core));
        }

        Some(parts.join("\n\n"))
    }
}

/// Response payload containing the assembled system prompt.
#[derive(Debug, Serialize, Deserialize)]
pub struct BuildResponse {
    /// The fully assembled system prompt string.
    pub prompt: String,
}

const TOOL_GUIDELINES: &str = "\
# Tool Usage Guidelines

## File Operations
- Always read a file before editing it.
- Prefer `edit_file` over `write_file` for existing files.
- Use `read_file` with offset/limit for large files.

## Search
- Use `glob` to find files by name pattern before using `grep` to search contents.
- Use `grep` with a file glob filter to narrow searches to relevant file types.

## Execution
- Use `bash` for git, build tools, package managers, and other terminal operations.
- Do NOT use `bash` for file operations — use the dedicated file tools.
- The bash working directory persists between calls.

## General
- Read before writing. Understand before changing.
- Make minimal, focused changes.";

#[capsule]
impl IdentityBuilder {
    /// Handles `identity.request.build` events. Reads workspace configuration
    /// and publishes the assembled system prompt to `identity.response.ready`.
    #[astrid::interceptor("handle_build_request")]
    pub fn build_system_prompt(&self, req: BuildRequest) -> Result<(), SysError> {
        let workspace_root = req.workspace_root.trim_end_matches('/');
        let project_name = workspace_root.split('/').last().unwrap_or("project");

        let opening = req
            .spark
            .as_ref()
            .and_then(|s| s.build_preamble())
            .unwrap_or_else(|| {
                format!(
                    "You are Astrid, working in the project \"{project_name}\"."
                )
            });

        let mut prompt = format!(
            "{opening}\n\n\
             # Environment\n\
             - Current working directory: {workspace_root}\n\
             - Platform: astrid-os\n\n"
        );

        prompt.push_str(TOOL_GUIDELINES);

        // Load project instructions (AGENTS.md or ASTRID.md)
        let agents_path = format!("{workspace_root}/AGENTS.md");
        if let Ok(content) = fs::read_string(&agents_path) {
            if !content.trim().is_empty() {
                prompt.push_str("\n\n# Agents Guidelines\n\n");
                prompt.push_str(&content);
            }
        } else {
            let astrid_path = format!("{workspace_root}/ASTRID.md");
            if let Ok(content) = fs::read_string(&astrid_path) {
                if !content.trim().is_empty() {
                    prompt.push_str("\n\n# Project Instructions\n\n");
                    prompt.push_str(&content);
                }
            }
        }

        // Load .astridignore workspace bounds
        let ignore_path = format!("{workspace_root}/.astridignore");
        if let Ok(content) = fs::read_string(&ignore_path) {
            if !content.trim().is_empty() {
                prompt.push_str("\n\n# Workspace Bounds (.astridignore)\n\n");
                prompt.push_str(&content);
            }
        }

        let response = BuildResponse { prompt };
        ipc::publish_json("identity.response.ready", &response)?;

        Ok(())
    }
}
