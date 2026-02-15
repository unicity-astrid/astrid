//! Hook definitions and types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use uuid::Uuid;

// Re-export HookEvent from astrid-core (canonical location).
pub use astrid_core::HookEvent;

/// Handler implementation for a hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum HookHandler {
    /// Execute a shell command.
    Command {
        /// The command to execute.
        command: String,
        /// Arguments to pass to the command.
        #[serde(default)]
        args: Vec<String>,
        /// Environment variables to set.
        #[serde(default)]
        env: HashMap<String, String>,
        /// Working directory for the command.
        #[serde(default)]
        working_dir: Option<String>,
    },
    /// Call an HTTP webhook.
    Http {
        /// The URL to call.
        url: String,
        /// HTTP method (GET, POST, etc.).
        #[serde(default = "default_http_method")]
        method: String,
        /// Headers to include.
        #[serde(default)]
        headers: HashMap<String, String>,
        /// Request body template.
        #[serde(default)]
        body_template: Option<String>,
    },
    /// Execute a WASM module via Extism.
    Wasm {
        /// Path to the WASM module.
        module_path: String,
        /// Function to call in the module.
        #[serde(default = "default_wasm_function")]
        function: String,
    },
    /// Invoke an LLM-based agent handler (stubbed - Phase 3).
    Agent {
        /// Agent prompt template.
        prompt_template: String,
        /// Model to use.
        #[serde(default)]
        model: Option<String>,
        /// Maximum tokens for response.
        #[serde(default)]
        max_tokens: Option<u32>,
    },
}

fn default_http_method() -> String {
    "POST".to_string()
}

fn default_wasm_function() -> String {
    "handle".to_string()
}

impl HookHandler {
    /// Create a new command handler.
    #[must_use]
    pub fn command(command: impl Into<String>) -> Self {
        Self::Command {
            command: command.into(),
            args: Vec::new(),
            env: HashMap::new(),
            working_dir: None,
        }
    }

    /// Create a new HTTP webhook handler.
    #[must_use]
    pub fn http(url: impl Into<String>) -> Self {
        Self::Http {
            url: url.into(),
            method: "POST".to_string(),
            headers: HashMap::new(),
            body_template: None,
        }
    }

    /// Create a new WASM handler.
    #[must_use]
    pub fn wasm(module_path: impl Into<String>) -> Self {
        Self::Wasm {
            module_path: module_path.into(),
            function: "handle".to_string(),
        }
    }

    /// Create a new agent handler (stubbed).
    #[must_use]
    pub fn agent(prompt_template: impl Into<String>) -> Self {
        Self::Agent {
            prompt_template: prompt_template.into(),
            model: None,
            max_tokens: None,
        }
    }

    /// Check if this handler is stubbed (not yet implemented).
    #[must_use]
    pub fn is_stubbed(&self) -> bool {
        matches!(self, Self::Agent { .. })
    }
}

/// Action to take when a hook fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailAction {
    /// Log a warning and continue.
    #[default]
    Warn,
    /// Block the operation that triggered the hook.
    Block,
    /// Silently ignore the failure.
    Ignore,
}

impl fmt::Display for FailAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Warn => write!(f, "warn"),
            Self::Block => write!(f, "block"),
            Self::Ignore => write!(f, "ignore"),
        }
    }
}

/// A hook definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hook {
    /// Unique identifier for this hook.
    pub id: Uuid,
    /// Human-readable name.
    #[serde(default)]
    pub name: Option<String>,
    /// Description of what this hook does.
    #[serde(default)]
    pub description: Option<String>,
    /// Event that triggers this hook.
    pub event: HookEvent,
    /// Optional matcher pattern (glob or regex).
    #[serde(default)]
    pub matcher: Option<HookMatcher>,
    /// Handler implementation.
    pub handler: HookHandler,
    /// Timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Action to take on failure.
    #[serde(default)]
    pub fail_action: FailAction,
    /// Run asynchronously (don't wait for completion).
    #[serde(default)]
    pub async_mode: bool,
    /// Whether the hook is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Priority (lower runs first).
    #[serde(default = "default_priority")]
    pub priority: i32,
}

fn default_timeout() -> u64 {
    30
}

fn default_enabled() -> bool {
    true
}

fn default_priority() -> i32 {
    100
}

impl Hook {
    /// Create a new hook for the given event.
    #[must_use]
    pub fn new(event: HookEvent) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: None,
            description: None,
            event,
            matcher: None,
            handler: HookHandler::command("echo"),
            timeout_secs: 30,
            fail_action: FailAction::Warn,
            async_mode: false,
            enabled: true,
            priority: 100,
        }
    }

    /// Set the hook's name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the hook's description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the handler for this hook.
    #[must_use]
    pub fn with_handler(mut self, handler: HookHandler) -> Self {
        self.handler = handler;
        self
    }

    /// Set a matcher pattern.
    #[must_use]
    pub fn with_matcher(mut self, matcher: HookMatcher) -> Self {
        self.matcher = Some(matcher);
        self
    }

    /// Set the timeout in seconds.
    #[must_use]
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Set the failure action.
    #[must_use]
    pub fn with_fail_action(mut self, action: FailAction) -> Self {
        self.fail_action = action;
        self
    }

    /// Enable async mode.
    #[must_use]
    pub fn async_mode(mut self) -> Self {
        self.async_mode = true;
        self
    }

    /// Disable the hook.
    #[must_use]
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Set the priority.
    #[must_use]
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}

/// Matcher for filtering when a hook should run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum HookMatcher {
    /// Match using a glob pattern.
    Glob {
        /// The glob pattern.
        pattern: String,
    },
    /// Match using a regex pattern.
    Regex {
        /// The regex pattern.
        pattern: String,
    },
    /// Match specific tool names.
    ToolNames {
        /// List of tool names to match.
        names: Vec<String>,
    },
    /// Match specific server names.
    ServerNames {
        /// List of server names to match.
        names: Vec<String>,
    },
}

impl HookMatcher {
    /// Create a glob matcher.
    #[must_use]
    pub fn glob(pattern: impl Into<String>) -> Self {
        Self::Glob {
            pattern: pattern.into(),
        }
    }

    /// Create a regex matcher.
    #[must_use]
    pub fn regex(pattern: impl Into<String>) -> Self {
        Self::Regex {
            pattern: pattern.into(),
        }
    }

    /// Create a tool names matcher.
    #[must_use]
    pub fn tools(names: Vec<String>) -> Self {
        Self::ToolNames { names }
    }

    /// Create a server names matcher.
    #[must_use]
    pub fn servers(names: Vec<String>) -> Self {
        Self::ServerNames { names }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_event_display() {
        assert_eq!(HookEvent::SessionStart.to_string(), "session_start");
        assert_eq!(HookEvent::PreToolCall.to_string(), "pre_tool_call");
    }

    #[test]
    fn test_hook_creation() {
        let hook = Hook::new(HookEvent::PreToolCall)
            .with_name("log-tool-calls")
            .with_handler(HookHandler::command("echo"))
            .with_timeout(60);

        assert_eq!(hook.event, HookEvent::PreToolCall);
        assert_eq!(hook.name, Some("log-tool-calls".to_string()));
        assert_eq!(hook.timeout_secs, 60);
        assert!(hook.enabled);
    }

    #[test]
    fn test_hook_handler_creation() {
        let cmd = HookHandler::command("echo");
        assert!(!cmd.is_stubbed());

        let wasm = HookHandler::wasm("/path/to/module.wasm");
        assert!(!wasm.is_stubbed());

        let agent = HookHandler::agent("Analyze this event: {{event}}");
        assert!(agent.is_stubbed());
    }

    #[test]
    fn test_hook_matcher() {
        let glob = HookMatcher::glob("fs_*");
        let regex = HookMatcher::regex(r"^fs_\w+$");
        let tools = HookMatcher::tools(vec!["read_file".to_string(), "write_file".to_string()]);

        assert!(matches!(glob, HookMatcher::Glob { .. }));
        assert!(matches!(regex, HookMatcher::Regex { .. }));
        assert!(matches!(tools, HookMatcher::ToolNames { .. }));
    }

    #[test]
    fn test_fail_action_default() {
        assert_eq!(FailAction::default(), FailAction::Warn);
    }
}
