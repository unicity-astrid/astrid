#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![allow(missing_docs)]

//! Shell execution tools capsule for Astrid OS.
//!
//! Provides the `run_shell_command` tool to agents, wrapping executions
//! securely in the host-level Escape Hatch (Seatbelt/bwrap).
//!
//! Each command is parsed to extract the program and subcommand, then
//! submitted for human approval before execution. Approved commands
//! create subcommand-level allowances (e.g. "git push" approves all
//! future `git push` variants).

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
const MULTI_COMMAND_PROGRAMS: &[&str] = &[
    "git", "docker", "kubectl", "npm", "npx", "yarn", "pnpm", "cargo", "pip", "pip3", "poetry",
    "systemctl", "brew", "apt", "apt-get", "dnf", "yum", "pacman", "snap", "flatpak", "helm",
    "terraform", "ansible", "vagrant", "make", "cmake", "go", "rustup",
];

/// Parse a shell command string into (program, optional subcommand).
///
/// For multi-command tools like git, returns ("git", Some("push")).
/// For simple tools like ls, returns ("ls", None).
fn parse_command(command: &str) -> (&str, Option<&str>) {
    let mut parts = command.split_whitespace();
    let program = match parts.next() {
        Some(p) => p,
        None => return ("", None),
    };

    if MULTI_COMMAND_PROGRAMS.contains(&program) {
        let subcommand = parts.next();
        (program, subcommand)
    } else {
        (program, None)
    }
}

/// Build the approval action string from parsed command parts.
fn approval_action(program: &str, subcommand: Option<&str>) -> String {
    match subcommand {
        Some(sub) => format!("{program} {sub}"),
        None => program.to_string(),
    }
}

#[expect(missing_docs)]
#[capsule]
impl ShellTools {
    /// Executes a given shell command via the host sandbox escape hatch.
    ///
    /// Before execution, parses the command to extract the program and
    /// subcommand, then requests human approval. If denied, returns an
    /// error without executing.
    #[astrid::tool("run_shell_command")]
    pub fn run_shell_command(&self, args: RunShellArgs) -> Result<String, SysError> {
        if args.command.trim().is_empty() {
            return Err(SysError::ApiError("Command cannot be empty".into()));
        }

        let (program, subcommand) = parse_command(&args.command);
        let action = approval_action(program, subcommand);

        // Request approval - blocks until the user responds or timeout.
        let result = approval::request(&action, &args.command, "high")?;
        if !result.approved {
            return Err(SysError::ApiError(format!(
                "Command '{}' was not approved by user",
                args.command
            )));
        }

        // Spawn the command via the SDK Process Airlock.
        // The core OS enforces the Capability and wraps it in bwrap/Seatbelt.
        let result = process::spawn("bash", &["-c", &args.command])?;

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

    #[test]
    fn parse_git_push() {
        let (prog, sub) = parse_command("git push origin main");
        assert_eq!(prog, "git");
        assert_eq!(sub, Some("push"));
    }

    #[test]
    fn parse_git_status() {
        let (prog, sub) = parse_command("git status");
        assert_eq!(prog, "git");
        assert_eq!(sub, Some("status"));
    }

    #[test]
    fn parse_docker_run() {
        let (prog, sub) = parse_command("docker run --rm alpine");
        assert_eq!(prog, "docker");
        assert_eq!(sub, Some("run"));
    }

    #[test]
    fn parse_ls_simple() {
        let (prog, sub) = parse_command("ls -la /tmp");
        assert_eq!(prog, "ls");
        assert_eq!(sub, None);
    }

    #[test]
    fn parse_empty_command() {
        let (prog, sub) = parse_command("");
        assert_eq!(prog, "");
        assert_eq!(sub, None);
    }

    #[test]
    fn parse_single_word_known() {
        let (prog, sub) = parse_command("git");
        assert_eq!(prog, "git");
        assert_eq!(sub, None);
    }

    #[test]
    fn approval_action_with_subcommand() {
        assert_eq!(approval_action("git", Some("push")), "git push");
    }

    #[test]
    fn approval_action_simple_program() {
        assert_eq!(approval_action("ls", None), "ls");
    }

    #[test]
    fn parse_cargo_build() {
        let (prog, sub) = parse_command("cargo build --release");
        assert_eq!(prog, "cargo");
        assert_eq!(sub, Some("build"));
    }

    #[test]
    fn parse_unknown_program_with_args() {
        let (prog, sub) = parse_command("my-custom-tool --flag value");
        assert_eq!(prog, "my-custom-tool");
        assert_eq!(sub, None);
    }
}
