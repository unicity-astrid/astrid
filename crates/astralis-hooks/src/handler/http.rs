//! HTTP webhook hook handler.
//!
//! Note: This is a minimal implementation that uses `curl` or similar
//! system commands. For production, consider using `reqwest` or similar.

use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, warn};

use super::{HandlerError, HandlerResult, parse_hook_result};
use crate::hook::HookHandler;
use crate::result::{HookContext, HookExecutionResult, HookResult};

/// Handler for HTTP webhooks.
#[derive(Debug, Clone, Default)]
pub struct HttpHandler;

impl HttpHandler {
    /// Create a new HTTP handler.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Execute an HTTP webhook handler.
    ///
    /// This implementation uses `curl` for simplicity. In production,
    /// you might want to use a proper HTTP client like `reqwest`.
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
        let HookHandler::Http {
            url,
            method,
            headers,
            body_template,
        } = handler
        else {
            return Err(HandlerError::InvalidConfiguration(
                "expected Http handler".to_string(),
            ));
        };

        debug!(url = %url, method = %method, "Executing HTTP hook");

        // Build curl command
        let mut cmd = Command::new("curl");
        cmd.arg("-s"); // Silent mode
        cmd.arg("-S"); // Show errors
        cmd.arg("-X").arg(method);

        // Add headers
        for (key, value) in headers {
            cmd.arg("-H").arg(format!("{key}: {value}"));
        }

        // Add content-type if posting JSON
        if body_template.is_some() {
            cmd.arg("-H").arg("Content-Type: application/json");
        }

        // Build body
        let body = if let Some(template) = body_template {
            // Simple template substitution
            substitute_template(template, context)
        } else {
            // Default: send context as JSON
            context.to_json().to_string()
        };

        if !body.is_empty() && (method == "POST" || method == "PUT" || method == "PATCH") {
            cmd.arg("-d").arg(&body);
        }

        cmd.arg(url);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Execute with timeout
        let output = match timeout(timeout_duration, cmd.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Ok(HookExecutionResult::Failure {
                    error: format!("Failed to execute curl: {e}"),
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
            warn!(
                url = %url,
                stderr = %stderr,
                "HTTP hook failed"
            );

            return Ok(HookExecutionResult::Failure {
                error: format!("HTTP request failed: {stderr}"),
                stderr: Some(stderr),
            });
        }

        // Parse the response
        let result = parse_hook_result(&stdout).unwrap_or_else(|e| {
            warn!(error = %e, "Failed to parse HTTP response, defaulting to Continue");
            HookResult::Continue
        });

        Ok(HookExecutionResult::Success {
            result,
            stdout: Some(stdout),
        })
    }
}

/// Escape a string for safe inclusion in JSON.
///
/// This prevents JSON injection attacks by properly escaping special characters.
fn escape_json_string(s: &str) -> String {
    use std::fmt::Write;

    let mut escaped = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            // Control characters U+0000 to U+001F
            c if c.is_control() => {
                let _ = write!(escaped, "\\u{:04x}", c as u32);
            },
            c => escaped.push(c),
        }
    }
    escaped
}

/// Simple template substitution with JSON escaping.
///
/// Supports `{{key}}` patterns where key is a data field in the context.
/// All values are JSON-escaped to prevent injection attacks.
fn substitute_template(template: &str, context: &HookContext) -> String {
    let mut result = template.to_string();

    // Replace standard fields (JSON-escaped)
    result = result.replace("{{event}}", &escape_json_string(&context.event.to_string()));
    result = result.replace(
        "{{invocation_id}}",
        &escape_json_string(&context.invocation_id.to_string()),
    );
    result = result.replace(
        "{{timestamp}}",
        &escape_json_string(&context.timestamp.to_rfc3339()),
    );

    if let Some(session_id) = &context.session_id {
        result = result.replace(
            "{{session_id}}",
            &escape_json_string(&session_id.to_string()),
        );
    }

    if let Some(user_id) = &context.user_id {
        result = result.replace("{{user_id}}", &escape_json_string(&user_id.to_string()));
    }

    // Replace data fields (JSON-escaped)
    for (key, value) in &context.data {
        let pattern = format!("{{{{{key}}}}}");
        let value_str = match value {
            serde_json::Value::String(s) => escape_json_string(s),
            other => escape_json_string(&other.to_string()),
        };
        result = result.replace(&pattern, &value_str);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::HookEvent;

    #[test]
    fn test_substitute_template() {
        let context = HookContext::new(HookEvent::PreToolCall)
            .with_data("tool_name", serde_json::json!("read_file"))
            .with_data("file_path", serde_json::json!("/home/user/test.txt"));

        let template =
            r#"{"event": "{{event}}", "tool": "{{tool_name}}", "path": "{{file_path}}"}"#;
        let result = substitute_template(template, &context);

        assert!(result.contains("pre_tool_call"));
        assert!(result.contains("read_file"));
        assert!(result.contains("/home/user/test.txt"));
    }

    #[test]
    fn test_json_escape_basic() {
        assert_eq!(escape_json_string("hello"), "hello");
        assert_eq!(escape_json_string("hello world"), "hello world");
    }

    #[test]
    fn test_json_escape_quotes() {
        assert_eq!(escape_json_string(r#"say "hello""#), r#"say \"hello\""#);
    }

    #[test]
    fn test_json_escape_backslash() {
        assert_eq!(escape_json_string(r"path\to\file"), r"path\\to\\file");
    }

    #[test]
    fn test_json_escape_control_chars() {
        assert_eq!(escape_json_string("line1\nline2"), r"line1\nline2");
        assert_eq!(escape_json_string("col1\tcol2"), r"col1\tcol2");
        assert_eq!(escape_json_string("a\rb"), r"a\rb");
    }

    #[test]
    fn test_json_escape_injection_prevention() {
        // Attempt to inject JSON
        let malicious = r#"value", "injected": "true"#;
        let escaped = escape_json_string(malicious);

        // The escaped string should be safe to include in JSON
        let json = format!(r#"{{"data": "{}"}}"#, escaped);

        // Should parse as valid JSON with escaped data
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed["data"].as_str().unwrap(),
            r#"value", "injected": "true"#
        );
    }

    #[test]
    fn test_substitute_template_with_injection_attempt() {
        let context = HookContext::new(HookEvent::PreToolCall).with_data(
            "malicious",
            serde_json::json!(r#"value", "extra": "injected"#),
        );

        let template = r#"{"data": "{{malicious}}"}"#;
        let result = substitute_template(template, &context);

        // The result should be valid JSON
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&result);
        assert!(parsed.is_ok());

        // The injected content should be escaped, not interpreted
        let value = parsed.unwrap();
        assert!(value["extra"].is_null()); // "extra" key should not exist
    }

    // Note: HTTP tests require a running server, so we skip them in unit tests.
    // Integration tests should cover actual HTTP calls.
}
