#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![allow(missing_docs)]

//! Shell execution tools capsule for Astrid OS.
//!
//! Provides the `run_shell_command` tool to agents, wrapping executions
//! securely in the host-level Escape Hatch (Seatbelt/bwrap).

use astrid_sdk::prelude::*;
use serde::Deserialize;

/// The main entry point for the Shell Tools capsule.
#[derive(Default)]
pub struct ShellTools;

/// Input arguments for the `run_shell_command` tool.
#[derive(Debug, Default, Deserialize)]
pub struct RunShellArgs {
    /// The exact bash command to execute.
    pub command: String,
}

#[allow(missing_docs)]
#[capsule]
impl ShellTools {
    /// Executes a given shell command via the host sandbox escape hatch.
    #[astrid::tool("run_shell_command")]
    pub fn run_shell_command(&self, args: RunShellArgs) -> Result<String, SysError> {
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
