#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![allow(missing_docs)]

//! Shell execution tools capsule for Astrid OS.
//!
//! Provides the `run_shell_command` tool to agents, wrapping executions
//! securely in the host-level Escape Hatch (Seatbelt/bwrap).
//!
//! Each command is parsed to extract an approval action from consecutive
//! non-flag tokens (up to 3 deep). Approved commands create allowances
//! at that granularity (e.g. "git push" approves all `git push` variants,
//! "docker compose up" approves all `docker compose up` variants).

use astrid_sdk::prelude::*;
use astrid_sdk::schemars;
use serde::Deserialize;

/// The main entry point for the Shell Tools capsule.
#[derive(Default)]
pub struct ShellTools;

/// Input arguments for the `run_shell_command` tool.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct RunShellArgs {
    /// The exact bash command to execute.
    pub command: String,
}

/// Maximum number of non-flag tokens to include in the approval action.
///
/// Covers sub-sub-commands like `docker compose up` (depth 3) without
/// pulling in positional arguments like remote names or file paths.
const MAX_ACTION_DEPTH: usize = 3;

/// Extract the approval action from a shell command string.
///
/// Collects consecutive non-flag tokens (up to [`MAX_ACTION_DEPTH`]),
/// stopping at the first token that starts with `-`. No whitelist needed -
/// all programs are treated uniformly.
///
/// ```text
/// git push --force origin main      -> "git push"
/// docker compose up -d              -> "docker compose up"
/// kubectl config set-context --cur  -> "kubectl config set-context"
/// ls -la /tmp                       -> "ls"
/// cargo build --release             -> "cargo build"
/// python -c "code"                  -> "python"
/// cat /etc/passwd                   -> "cat /etc/passwd"
/// ```
fn extract_action(command: &str) -> String {
    command
        .split_whitespace()
        .take(MAX_ACTION_DEPTH)
        .take_while(|t| !t.starts_with('-'))
        .collect::<Vec<_>>()
        .join(" ")
}

#[expect(missing_docs)]
#[capsule]
impl ShellTools {
    /// Executes a given shell command via the host sandbox escape hatch.
    ///
    /// Before execution, extracts the approval action (consecutive non-flag
    /// tokens, up to 3 deep), then requests human approval. If denied,
    /// returns an error without executing.
    #[astrid::tool("run_shell_command")]
    pub fn run_shell_command(&self, args: RunShellArgs) -> Result<String, SysError> {
        let trimmed = args.command.trim();
        if trimmed.is_empty() {
            return Err(SysError::ApiError("Command cannot be empty".into()));
        }

        let action = extract_action(trimmed);

        // Request approval - blocks until the user responds or timeout.
        let result = approval::request(&action, trimmed, "high")?;
        if !result.approved {
            return Err(SysError::ApiError(format!(
                "Command '{trimmed}' was not approved by user",
            )));
        }

        // Spawn the command via the SDK Process Airlock.
        // The core OS enforces the Capability and wraps it in bwrap/Seatbelt.
        let result = process::spawn("bash", &["-c", trimmed])?;

        // If the command fails, we return the stderr as an API error so the LLM knows it failed.
        if result.exit_code != 0 {
            return Err(SysError::ApiError(format!(
                "Command failed with exit code {}: {}",
                result.exit_code, result.stderr
            )));
        }

        // Return stdout back to the LLM agent
        Ok(result.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Subcommand extraction (depth 1) --

    #[test]
    fn action_git_push() {
        assert_eq!(extract_action("git push --force origin main"), "git push");
    }

    #[test]
    fn action_git_status() {
        assert_eq!(extract_action("git status"), "git status");
    }

    #[test]
    fn action_cargo_build() {
        assert_eq!(extract_action("cargo build --release"), "cargo build");
    }

    // -- Sub-sub-command extraction (depth 2) --

    #[test]
    fn action_docker_compose_up() {
        assert_eq!(extract_action("docker compose up -d"), "docker compose up");
    }

    #[test]
    fn action_kubectl_config_set_context() {
        assert_eq!(
            extract_action("kubectl config set-context --current"),
            "kubectl config set-context"
        );
    }

    #[test]
    fn action_git_remote_add() {
        assert_eq!(
            extract_action("git remote add origin https://example.com"),
            "git remote add"
        );
    }

    // -- Depth cap (stops at 3 tokens) --

    #[test]
    fn action_depth_cap() {
        // 4th non-flag token is NOT included
        assert_eq!(extract_action("a b c d e"), "a b c");
    }

    // -- Flag stops extraction --

    #[test]
    fn action_flag_stops_immediately() {
        assert_eq!(extract_action("ls -la /tmp"), "ls");
    }

    #[test]
    fn action_python_flag() {
        assert_eq!(extract_action("python -c 'print(1)'"), "python");
    }

    // -- Simple programs (no flags, bare args) --

    #[test]
    fn action_cat_with_path() {
        assert_eq!(extract_action("cat /etc/passwd"), "cat /etc/passwd");
    }

    #[test]
    fn action_unknown_tool_with_flag() {
        assert_eq!(extract_action("my-tool --flag value"), "my-tool");
    }

    // -- Edge cases --

    #[test]
    fn action_empty() {
        assert_eq!(extract_action(""), "");
    }

    #[test]
    fn action_single_word() {
        assert_eq!(extract_action("git"), "git");
    }

    #[test]
    fn action_only_flags() {
        assert_eq!(extract_action("--help"), "");
    }
}
