#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

//! Identity Builder Capsule for Astrid OS.
//!
//! Subscribes to IPC events to generate the system prompt for the LLM agent.

use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Default)]
pub struct IdentityBuilder;

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct BuildRequest {
    pub workspace_root: String,
    pub spark: Option<SparkConfig>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SparkConfig {
    pub callsign: String,
    pub class: String,
    pub aura: String,
    pub signal: String,
    pub core: String,
}

impl SparkConfig {
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

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildResponse {
    pub prompt: String,
}

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

#[capsule]
impl IdentityBuilder {
    #[astrid::interceptor("handle_build_request")]
    pub fn build_system_prompt(&self, req: BuildRequest) -> Result<(), SysError> {
        let os = "astrid-os"; // or get from sys::get_config_string?
        let workspace_root = req.workspace_root.trim_end_matches('/');
        let project_name = workspace_root.split('/').last().unwrap_or("project");

        let opening = req.spark.as_ref().and_then(|s| s.build_preamble()).unwrap_or_else(|| {
            format!("You are Astrid, working in the project \"{}\".", project_name)
        });

        let mut prompt = format!(
            "{}\n\n\
             # Environment\n\
             - Current working directory: {}\n\
             - Platform: {}\n\n",
            opening, workspace_root, os
        );

        prompt.push_str(TOOL_GUIDELINES);

        // Load project instructions (AGENTS.md / ASTRID.md)
        let agents_path = format!("{}/AGENTS.md", workspace_root);
        if let Ok(content) = fs::read_string(&agents_path) {
            if !content.trim().is_empty() {
                prompt.push_str("\n\n# Agents Guidelines\n\n");
                prompt.push_str(&content);
            }
        } else {
            let astrid_path = format!("{}/ASTRID.md", workspace_root);
            if let Ok(content) = fs::read_string(&astrid_path) {
                if !content.trim().is_empty() {
                    prompt.push_str("\n\n# Project Instructions\n\n");
                    prompt.push_str(&content);
                }
            }
        }

        // Parse .astridignore bounds
        let ignore_path = format!("{}/.astridignore", workspace_root);
        if let Ok(content) = fs::read_string(&ignore_path) {
            if !content.trim().is_empty() {
                prompt.push_str("\n\n# Workspace Bounds (.astridignore)\n\n");
                prompt.push_str(&content);
            }
        }

        let response = BuildResponse { prompt };

        // Publish over Event Bus via IPC
        let _ = ipc::publish_json("identity.response.ready", &response);

        Ok(())
    }
}
