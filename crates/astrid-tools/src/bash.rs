//! Bash tool — executes shell commands with persistent working directory.

use crate::{BuiltinTool, ToolContext, ToolError, ToolResult};
use serde_json::Value;
use std::path::PathBuf;
use tokio::process::Command;
use uuid::Uuid;

/// Default timeout in milliseconds (2 minutes).
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
/// Maximum timeout in milliseconds (10 minutes).
const MAX_TIMEOUT_MS: u64 = 600_000;

/// Built-in tool for executing bash commands.
pub struct BashTool;

#[async_trait::async_trait]
impl BuiltinTool for BashTool {
    fn name(&self) -> &'static str {
        "bash"
    }

    fn description(&self) -> &'static str {
        "Executes a bash command. The working directory persists between invocations. \
         Use for git, npm, cargo, docker, and other terminal operations. \
         Optional timeout in milliseconds (max 600000)."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default: 120000, max: 600000)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let command = args
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArguments("command is required".into()))?;

        let timeout_ms = args
            .get("timeout")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        let cwd = ctx.cwd.read().await.clone();

        // Use a per-invocation UUID as the CWD sentinel. A fixed sentinel string
        // could be injected by command output (e.g. `echo __ASTRID_CWD__; echo /evil`),
        // poisoning the shared `cwd`. A UUID is unguessable, so command output cannot
        // replicate it and claim a false working directory.
        let sentinel = Uuid::new_v4().to_string();
        let wrapped = format!(
            "{command}\n__ASTRID_EXIT__=$?\necho \"{sentinel}\"\npwd\nexit $__ASTRID_EXIT__"
        );

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            run_bash(&wrapped, &cwd),
        )
        .await;

        match result {
            Ok(Ok((stdout, stderr, exit_code))) => {
                // Parse stdout: split on sentinel to get output and new cwd
                let (output, new_cwd) = parse_sentinel_output(&stdout, &sentinel);

                // Update persistent cwd
                if let Some(new_cwd) = new_cwd {
                    let mut cwd_lock = ctx.cwd.write().await;
                    *cwd_lock = new_cwd;
                }

                let mut result_text = String::new();

                if !output.is_empty() {
                    result_text.push_str(&output);
                }

                if !stderr.is_empty() {
                    if !result_text.is_empty() {
                        result_text.push('\n');
                    }
                    result_text.push_str("STDERR:\n");
                    result_text.push_str(&stderr);
                }

                if exit_code != 0 {
                    if !result_text.is_empty() {
                        result_text.push('\n');
                    }
                    result_text.push_str("(exit code: ");
                    result_text.push_str(&exit_code.to_string());
                    result_text.push(')');
                }

                if result_text.is_empty() {
                    result_text.push_str("(no output)");
                }

                Ok(result_text)
            },
            Ok(Err(e)) => Err(ToolError::ExecutionFailed(e.to_string())),
            Err(_) => Err(ToolError::Timeout(timeout_ms)),
        }
    }
}

/// Run a bash command and capture stdout, stderr, and exit code.
async fn run_bash(command: &str, cwd: &std::path::Path) -> std::io::Result<(String, String, i32)> {
    let output = Command::new("bash")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok((stdout, stderr, exit_code))
}

/// Parse the sentinel from stdout to extract command output and new cwd.
///
/// `sentinel` must be the per-invocation UUID generated before the command ran.
/// Because the sentinel is unguessable, only the line appended by our shell wrapper
/// matches; any matching line produced by command output would require the process
/// to have known the UUID in advance.
fn parse_sentinel_output(stdout: &str, sentinel: &str) -> (String, Option<PathBuf>) {
    if let Some(sentinel_pos) = stdout.find(sentinel) {
        let output = stdout[..sentinel_pos].trim_end().to_string();
        #[allow(clippy::arithmetic_side_effects)]
        let after_sentinel = &stdout[sentinel_pos + sentinel.len()..];
        let new_cwd = after_sentinel
            .lines()
            .find(|l| !l.is_empty())
            .map(|l| PathBuf::from(l.trim()));
        (output, new_cwd)
    } else {
        (stdout.to_string(), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ctx_with_root(root: &std::path::Path) -> ToolContext {
        ToolContext::new(root.to_path_buf(), None)
    }

    #[tokio::test]
    async fn test_bash_echo() {
        let ctx = ctx_with_root(&std::env::temp_dir());
        let result = BashTool
            .execute(serde_json::json!({"command": "echo hello"}), &ctx)
            .await
            .unwrap();

        assert!(result.contains("hello"));
    }

    #[tokio::test]
    async fn test_bash_exit_code() {
        let ctx = ctx_with_root(&std::env::temp_dir());
        let result = BashTool
            .execute(serde_json::json!({"command": "exit 42"}), &ctx)
            .await
            .unwrap();

        assert!(result.contains("exit code: 42"));
    }

    #[tokio::test]
    async fn test_bash_stderr() {
        let ctx = ctx_with_root(&std::env::temp_dir());
        let result = BashTool
            .execute(serde_json::json!({"command": "echo error >&2"}), &ctx)
            .await
            .unwrap();

        assert!(result.contains("STDERR:"));
        assert!(result.contains("error"));
    }

    #[tokio::test]
    async fn test_bash_cwd_persistence() {
        let dir = TempDir::new().unwrap();
        let ctx = ctx_with_root(dir.path());

        // Create a subdirectory and cd into it
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        BashTool
            .execute(serde_json::json!({"command": "cd subdir"}), &ctx)
            .await
            .unwrap();

        // Verify cwd was updated
        let cwd = ctx.cwd.read().await.clone();
        assert!(cwd.ends_with("subdir"));

        // Next command should run in the new cwd
        let result = BashTool
            .execute(serde_json::json!({"command": "pwd"}), &ctx)
            .await
            .unwrap();

        assert!(result.contains("subdir"));
    }

    #[tokio::test]
    async fn test_bash_timeout() {
        let ctx = ctx_with_root(&std::env::temp_dir());
        let result = BashTool
            .execute(
                serde_json::json!({"command": "sleep 10", "timeout": 100}),
                &ctx,
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolError::Timeout(100)));
    }

    #[test]
    fn test_parse_sentinel_output() {
        let sentinel = "550e8400-e29b-41d4-a716-446655440000";
        let stdout = format!("hello world\n{sentinel}\n/tmp/test\n");
        let (output, cwd) = parse_sentinel_output(&stdout, sentinel);
        assert_eq!(output, "hello world");
        assert_eq!(cwd, Some(PathBuf::from("/tmp/test")));
    }

    #[test]
    fn test_parse_sentinel_no_sentinel() {
        let sentinel = "550e8400-e29b-41d4-a716-446655440000";
        let (output, cwd) = parse_sentinel_output("hello world\n", sentinel);
        assert_eq!(output, "hello world\n");
        assert!(cwd.is_none());
    }

    #[test]
    fn test_parse_sentinel_injection_blocked() {
        // Simulates a command that outputs the old static sentinel followed by a crafted path.
        // With a per-invocation UUID, the injected string cannot match the real sentinel.
        let sentinel = "550e8400-e29b-41d4-a716-446655440000";
        let injected = "__ASTRID_CWD__";
        let stdout = format!("{injected}\n/evil\n{sentinel}\n/actual/cwd\n");
        let (output, cwd) = parse_sentinel_output(&stdout, sentinel);
        // The injected sentinel appears in output (it's just text), not as the CWD.
        assert!(output.contains(injected));
        assert_eq!(cwd, Some(PathBuf::from("/actual/cwd")));
    }
}
