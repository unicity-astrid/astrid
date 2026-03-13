#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![allow(missing_docs)]

//! Shell execution tools capsule for Astrid OS.
//!
//! Provides the `run_shell_command` tool to agents, wrapping executions
//! securely in the host-level Escape Hatch (Seatbelt/bwrap).
//!
//! Each command is parsed to extract an approval action: the program name
//! plus subcommand tokens for known multi-command tools. Approved commands
//! create allowances at that granularity (e.g. "git push" approves all
//! `git push` variants, "docker compose up" approves all
//! `docker compose up` variants). Unknown programs use program-name-only
//! to avoid leaking positional arguments into allowance patterns.

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

/// Programs known to have meaningful subcommands for approval grouping.
///
/// For listed programs, non-flag tokens after the program name are extracted
/// as subcommands (up to 2 levels deep). For unlisted programs, only the
/// program name is used as the action to avoid leaking positional arguments
/// (e.g. file paths) into allowance patterns.
const MULTI_COMMAND_PROGRAMS: &[&str] = &[
    "git", "docker", "kubectl", "npm", "npx", "yarn", "pnpm", "cargo", "pip", "pip3", "poetry",
    "systemctl", "brew", "apt", "apt-get", "dnf", "yum", "pacman", "snap", "flatpak", "helm",
    "terraform", "ansible", "vagrant", "make", "cmake", "go", "rustup", "bun", "deno", "uv",
    "nix", "podman",
];

/// Maximum subcommand depth for known multi-command programs.
///
/// Covers sub-sub-commands like `docker compose up` (program + 2 subs)
/// without pulling in positional arguments.
const MAX_SUBCOMMAND_DEPTH: usize = 2;

/// Extract the approval action from a shell command string.
///
/// For known multi-command programs, collects the program name plus up to
/// [`MAX_SUBCOMMAND_DEPTH`] non-flag subcommand tokens. For unknown
/// programs, returns just the program name.
///
/// ```text
/// git push --force origin main      -> "git push"
/// docker compose up -d              -> "docker compose up"
/// kubectl config set-context --cur  -> "kubectl config set-context"
/// ls -la /tmp                       -> "ls"
/// cargo build --release             -> "cargo build"
/// python -c "code"                  -> "python"
/// cat /etc/passwd                   -> "cat"
/// rm -rf /tmp/foo                   -> "rm"
/// rm /tmp/foo                       -> "rm"
/// ```
fn extract_action(command: &str) -> String {
    let mut tokens = command.split_whitespace();
    let program = match tokens.next() {
        Some(p) if !p.starts_with('-') => p,
        _ => return String::new(),
    };

    if !MULTI_COMMAND_PROGRAMS.contains(&program) {
        return program.to_string();
    }

    // Known multi-command program: extract non-flag subcommands.
    let mut parts = vec![program];
    for token in tokens {
        if token.starts_with('-') || parts.len() > MAX_SUBCOMMAND_DEPTH {
            break;
        }
        parts.push(token);
    }
    parts.join(" ")
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

    // -- Depth cap (stops at MAX_SUBCOMMAND_DEPTH + 1 tokens for known programs) --

    #[test]
    fn action_depth_cap_known_program() {
        // npm is in the whitelist; 3rd non-flag token is NOT included
        assert_eq!(extract_action("npm run build dist"), "npm run build");
    }

    // -- Unknown programs return name only --

    #[test]
    fn action_unknown_program_returns_name_only() {
        // Unknown programs never include arguments in the action
        assert_eq!(extract_action("cat /etc/passwd"), "cat");
        assert_eq!(extract_action("rm /tmp/foo"), "rm");
        assert_eq!(extract_action("rm -rf /tmp/foo"), "rm");
        assert_eq!(extract_action("a b c d e"), "a");
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
