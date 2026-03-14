#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![warn(missing_docs)]

//! Cross-session memory capsule for Astrid OS.
//!
//! Hooks into the prompt builder pipeline via
//! `prompt_builder.v1.hook.before_build` and appends the contents of
//! `.astrid/memory.md` to the system prompt using `appendSystemContext`.
//!
//! The agent maintains this file using existing `write_file` /
//! `replace_in_file` tools from `astrid-capsule-fs`. No new tools are
//! needed - this capsule is read-only.

use astrid_sdk::prelude::*;

/// Maximum size in bytes for the cross-session memory file.
///
/// Prevents unbounded context window consumption from agent-written
/// content. Unlike AGENTS.md (human-authored), memory.md is written by
/// the agent and can grow without limit.
const MAX_MEMORY_BYTES: usize = 32_768;

/// Path to the memory file relative to the workspace root.
///
/// The VFS strips absolute prefixes via `make_relative()` and resolves
/// against the workspace root, so a relative path works correctly.
const MEMORY_PATH: &str = ".astrid/memory.md";

/// Cross-session memory injector capsule.
#[derive(Default)]
pub struct MemoryInjector;

#[capsule]
impl MemoryInjector {
    /// Intercepts `prompt_builder.v1.hook.before_build` events.
    ///
    /// Reads `.astrid/memory.md` from the workspace and publishes a
    /// hook response with `appendSystemContext` on the response topic.
    /// If the file is missing or empty, this is a no-op.
    #[astrid::interceptor("on_before_prompt_build")]
    pub fn on_before_prompt_build(
        &self,
        payload: serde_json::Value,
    ) -> Result<(), SysError> {
        let response_topic = payload
            .get("response_topic")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SysError::ApiError("missing response_topic in before_build payload".into())
            })?;

        let content = match fs::read_to_string(MEMORY_PATH) {
            Ok(c) if !c.trim().is_empty() => c,
            _ => return Ok(()),
        };

        let section = if content.len() > MAX_MEMORY_BYTES {
            let end = content.floor_char_boundary(MAX_MEMORY_BYTES);
            format!("# Memory\n\n{}\n\n[Memory truncated]", &content[..end])
        } else {
            format!("# Memory\n\n{content}")
        };

        ipc::publish_json(
            response_topic,
            &serde_json::json!({ "appendSystemContext": section }),
        )?;

        Ok(())
    }
}
