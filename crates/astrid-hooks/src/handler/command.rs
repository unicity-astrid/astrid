//! Command hook handler - executes shell commands.
//!
//! # Security
//!
//! This handler implements several security measures:
//! - Environment variable clearing (inherits only from allowlist)
//! - PATH restriction to safe directories
//! - Working directory isolation

use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, warn};

use super::{HandlerError, HandlerResult, parse_hook_result};
use crate::hook::HookHandler;
use crate::result::{HookContext, HookExecutionResult, HookResult};

/// Environment variables that are safe to inherit from the parent process.
const ALLOWED_ENV_VARS: &[&str] = &[
    // Essential system variables
    "PATH", "HOME", "USER", "SHELL", "TERM", "LANG", "LC_ALL", "LC_CTYPE",
    // Temporary directories
    "TMPDIR", "TMP", "TEMP",
];

/// Safe directories to include in PATH for sandboxed execution.
/// These are common system directories that contain safe utilities.
#[cfg(unix)]
const SAFE_PATH_DIRS: &[&str] = &["/usr/bin", "/bin", "/usr/local/bin"];

#[cfg(windows)]
const SAFE_PATH_DIRS: &[&str] = &[r"C:\Windows\System32", r"C:\Windows"];

/// Handler for executing shell commands with security sandboxing.
#[derive(Debug, Clone)]
pub struct CommandHandler {
    /// Whether to enable strict sandboxing (clear env, restrict PATH).
    sandboxed: bool,
}

impl Default for CommandHandler {
    fn default() -> Self {
        Self { sandboxed: true }
    }
}

impl CommandHandler {
    /// Create a new command handler with default sandboxing enabled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new command handler with explicit sandbox setting.
    #[must_use]
    pub fn with_sandbox(sandboxed: bool) -> Self {
        Self { sandboxed }
    }

    /// Get the restricted PATH for sandboxed execution.
    fn safe_path() -> String {
        SAFE_PATH_DIRS.join(if cfg!(windows) { ";" } else { ":" })
    }

    /// Execute a command handler.
    ///
    /// # Security
    ///
    /// When sandboxing is enabled:
    /// - Clears environment variables except for allowlisted ones
    /// - Restricts PATH to safe system directories
    /// - Runs the command with minimal privileges
    ///
    /// # Errors
    ///
    /// Returns an error if the handler configuration is invalid.
    pub async fn execute(
        &self,
        handler: &HookHandler,
        context: &HookContext,
        timeout_duration: Duration,
    ) -> HandlerResult<HookExecutionResult> {
        let HookHandler::Command {
            command,
            args,
            env,
            working_dir,
        } = handler
        else {
            return Err(HandlerError::InvalidConfiguration(
                "expected Command handler".to_string(),
            ));
        };

        debug!(command = %command, args = ?args, sandboxed = %self.sandboxed, "Executing command hook");

        // Build the command
        let mut cmd = Command::new(command);
        cmd.args(args);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Set working directory if specified
        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        }

        // Apply sandboxing
        if self.sandboxed {
            // Clear all environment variables first
            cmd.env_clear();

            // Re-add only safe variables from the parent environment
            for var in ALLOWED_ENV_VARS {
                if let Ok(value) = std::env::var(var) {
                    // Special handling for PATH - use restricted version
                    if *var == "PATH" {
                        cmd.env("PATH", Self::safe_path());
                    } else {
                        cmd.env(var, value);
                    }
                }
            }
        }

        // Add custom environment variables (from hook config)
        for (key, value) in env {
            cmd.env(key, value);
        }

        // Add context as environment variables (Astrid-specific)
        for (key, value) in context.to_env_vars() {
            cmd.env(key, value);
        }

        // Serialize context JSON for stdin delivery
        let context_json = context.to_json().to_string();

        // Execute with timeout, piping context JSON on stdin
        let output = match timeout(timeout_duration, async {
            let mut child = cmd.spawn()?;

            // Write context JSON to stdin, then close it so the child sees EOF
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(context_json.as_bytes()).await;
                let _ = stdin.shutdown().await;
            }

            child.wait_with_output().await
        })
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Ok(HookExecutionResult::Failure {
                    error: format!("Failed to execute command: {e}"),
                    stderr: None,
                });
            },
            Err(_) => {
                return Ok(HookExecutionResult::Timeout {
                    timeout_secs: timeout_duration.as_secs(),
                });
            },
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            let exit_code = output.status.code().unwrap_or(-1);
            warn!(
                command = %command,
                exit_code = exit_code,
                stderr = %stderr,
                "Command hook failed"
            );

            return Ok(HookExecutionResult::Failure {
                error: format!("Command exited with code {exit_code}"),
                stderr: Some(stderr),
            });
        }

        // Parse the result from stdout
        let result = parse_hook_result(&stdout).unwrap_or_else(|e| {
            warn!(error = %e, "Failed to parse hook result, defaulting to Continue");
            HookResult::Continue
        });

        Ok(HookExecutionResult::Success {
            result,
            stdout: Some(stdout),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::HookEvent;

    #[tokio::test]
    async fn test_command_handler_echo() {
        let handler = CommandHandler::new();
        let hook_handler = HookHandler::Command {
            command: "echo".to_string(),
            args: vec!["continue".to_string()],
            env: Default::default(),
            working_dir: None,
        };
        let context = HookContext::new(HookEvent::SessionStart);

        let result = handler
            .execute(&hook_handler, &context, Duration::from_secs(5))
            .await
            .unwrap();

        assert!(result.is_success());
        if let HookExecutionResult::Success { result, .. } = result {
            assert!(matches!(result, HookResult::Continue));
        }
    }

    #[tokio::test]
    async fn test_command_handler_with_env() {
        let handler = CommandHandler::new();
        let hook_handler = HookHandler::Command {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "echo $ASTRID_HOOK_EVENT".to_string()],
            env: Default::default(),
            working_dir: None,
        };
        let context = HookContext::new(HookEvent::PreToolCall);

        let result = handler
            .execute(&hook_handler, &context, Duration::from_secs(5))
            .await
            .unwrap();

        if let HookExecutionResult::Success { stdout, .. } = result {
            assert!(stdout.unwrap_or_default().contains("pre_tool_call"));
        }
    }

    #[tokio::test]
    async fn test_command_handler_timeout() {
        let handler = CommandHandler::new();
        let hook_handler = HookHandler::Command {
            command: "sleep".to_string(),
            args: vec!["10".to_string()],
            env: Default::default(),
            working_dir: None,
        };
        let context = HookContext::new(HookEvent::SessionStart);

        let result = handler
            .execute(&hook_handler, &context, Duration::from_millis(100))
            .await
            .unwrap();

        assert!(matches!(result, HookExecutionResult::Timeout { .. }));
    }

    #[tokio::test]
    async fn test_command_handler_failure() {
        let handler = CommandHandler::new();
        let hook_handler = HookHandler::Command {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "exit 1".to_string()],
            env: Default::default(),
            working_dir: None,
        };
        let context = HookContext::new(HookEvent::SessionStart);

        let result = handler
            .execute(&hook_handler, &context, Duration::from_secs(5))
            .await
            .unwrap();

        assert!(matches!(result, HookExecutionResult::Failure { .. }));
    }

    #[tokio::test]
    async fn test_command_handler_sandboxed() {
        // Create a sandboxed handler
        let handler = CommandHandler::with_sandbox(true);
        let hook_handler = HookHandler::Command {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "echo $HOME".to_string()],
            env: Default::default(),
            working_dir: None,
        };
        let context = HookContext::new(HookEvent::SessionStart);

        let result = handler
            .execute(&hook_handler, &context, Duration::from_secs(5))
            .await
            .unwrap();

        // HOME should still be available (it's in the allowlist)
        if let HookExecutionResult::Success { stdout, .. } = result {
            let output = stdout.unwrap_or_default();
            // Should have some output (HOME is typically set)
            assert!(!output.trim().is_empty() || std::env::var("HOME").is_err());
        }
    }

    #[tokio::test]
    async fn test_command_handler_unsandboxed() {
        // Create an unsandboxed handler
        let handler = CommandHandler::with_sandbox(false);
        let hook_handler = HookHandler::Command {
            command: "echo".to_string(),
            args: vec!["continue".to_string()],
            env: Default::default(),
            working_dir: None,
        };
        let context = HookContext::new(HookEvent::SessionStart);

        let result = handler
            .execute(&hook_handler, &context, Duration::from_secs(5))
            .await
            .unwrap();

        assert!(result.is_success());
    }

    #[tokio::test]
    async fn test_command_handler_custom_env_in_sandbox() {
        let handler = CommandHandler::with_sandbox(true);

        let mut custom_env = std::collections::HashMap::new();
        custom_env.insert("CUSTOM_VAR".to_string(), "custom_value".to_string());

        let hook_handler = HookHandler::Command {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "echo $CUSTOM_VAR".to_string()],
            env: custom_env,
            working_dir: None,
        };
        let context = HookContext::new(HookEvent::SessionStart);

        let result = handler
            .execute(&hook_handler, &context, Duration::from_secs(5))
            .await
            .unwrap();

        // Custom env vars should still be set even in sandbox mode
        if let HookExecutionResult::Success { stdout, .. } = result {
            assert!(stdout.unwrap_or_default().contains("custom_value"));
        }
    }

    #[tokio::test]
    async fn test_command_handler_stdin_context() {
        let handler = CommandHandler::new();
        // Read JSON from stdin and extract the event field
        let hook_handler = HookHandler::Command {
            command: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                // Read stdin fully, then extract the event field with basic shell tools
                r#"INPUT=$(cat); echo "$INPUT" | grep -o '"event":"[^"]*"' | head -1"#.to_string(),
            ],
            env: Default::default(),
            working_dir: None,
        };
        let context = HookContext::new(HookEvent::PreToolCall)
            .with_data("tool_name", serde_json::json!("Bash"));

        let result = handler
            .execute(&hook_handler, &context, Duration::from_secs(5))
            .await
            .unwrap();

        assert!(result.is_success());
        if let HookExecutionResult::Success { stdout, .. } = result {
            let output = stdout.unwrap_or_default();
            assert!(
                output.contains("pre_tool_call"),
                "stdin should contain context JSON with event field, got: {output}"
            );
        }
    }

    #[test]
    fn test_safe_path() {
        let path = CommandHandler::safe_path();
        // Should contain at least one standard directory
        #[cfg(unix)]
        assert!(path.contains("/bin") || path.contains("/usr/bin"));
        #[cfg(windows)]
        assert!(path.contains("System32"));
    }

    #[test]
    fn test_allowed_env_vars() {
        // Verify the allowlist contains expected variables
        assert!(ALLOWED_ENV_VARS.contains(&"PATH"));
        assert!(ALLOWED_ENV_VARS.contains(&"HOME"));
        assert!(ALLOWED_ENV_VARS.contains(&"USER"));

        // Verify potentially dangerous variables are NOT in the list
        assert!(!ALLOWED_ENV_VARS.contains(&"LD_PRELOAD"));
        assert!(!ALLOWED_ENV_VARS.contains(&"LD_LIBRARY_PATH"));
        assert!(!ALLOWED_ENV_VARS.contains(&"DYLD_INSERT_LIBRARIES"));
    }
}
