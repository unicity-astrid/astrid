//! Message routing for the gateway.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Channel binding configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelBinding {
    /// Agent ID this binding routes to.
    pub agent_id: String,

    /// Channel type (cli, discord, web, telegram, etc.).
    pub channel_type: String,

    /// Scope within the channel (dm, guild, channel, etc.).
    pub scope: Option<String>,

    /// Identifier pattern (user id, guild id, room id, etc.).
    pub identifier: Option<String>,

    /// Priority for matching (higher = preferred).
    #[serde(default)]
    pub priority: i32,
}

impl ChannelBinding {
    /// Create a new channel binding.
    #[must_use]
    pub fn new(agent_id: impl Into<String>, channel_type: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            channel_type: channel_type.into(),
            scope: None,
            identifier: None,
            priority: 0,
        }
    }

    /// Set the scope.
    #[must_use]
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    /// Set the identifier.
    #[must_use]
    pub fn with_identifier(mut self, identifier: impl Into<String>) -> Self {
        self.identifier = Some(identifier.into());
        self
    }

    /// Set the priority.
    #[must_use]
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Check if this binding matches a route key.
    #[must_use]
    pub fn matches(
        &self,
        channel_type: &str,
        scope: Option<&str>,
        identifier: Option<&str>,
    ) -> bool {
        if self.channel_type != channel_type {
            return false;
        }

        match (&self.scope, scope) {
            (Some(binding_scope), Some(route_scope)) => {
                if binding_scope != route_scope && binding_scope != "*" {
                    return false;
                }
            },
            (Some(_), None) => return false,
            (None, _) => {}, // No scope requirement matches any
        }

        match (&self.identifier, identifier) {
            (Some(binding_id), Some(route_id)) => {
                if binding_id != route_id && binding_id != "*" {
                    return false;
                }
            },
            (Some(_), None) => return false,
            (None, _) => {}, // No identifier requirement matches any
        }

        true
    }
}

/// Message router for directing messages to agents.
#[derive(Debug, Default)]
pub struct MessageRouter {
    /// Registered channel bindings.
    bindings: Vec<ChannelBinding>,

    /// Cache of session keys to agent IDs.
    session_cache: HashMap<String, String>,
}

impl MessageRouter {
    /// Create a new message router.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a channel binding.
    pub fn register(&mut self, binding: ChannelBinding) {
        self.bindings.push(binding);
        // Re-sort by priority (highest first)
        self.bindings.sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    /// Remove bindings for an agent.
    pub fn unregister(&mut self, agent_id: &str) {
        self.bindings.retain(|b| b.agent_id != agent_id);
        self.session_cache.retain(|_, v| v != agent_id);
    }

    /// Generate a session key.
    ///
    /// Format: `agent:{agent_id}:{channel}:{scope}:{identifier}`
    #[must_use]
    pub fn session_key(
        agent_id: &str,
        channel: &str,
        scope: Option<&str>,
        identifier: Option<&str>,
    ) -> String {
        format!(
            "agent:{}:{}:{}:{}",
            agent_id,
            channel,
            scope.unwrap_or("_"),
            identifier.unwrap_or("_")
        )
    }

    /// Route a message to an agent.
    ///
    /// Returns the agent ID and session key if a matching binding is found.
    #[must_use]
    pub fn route(
        &mut self,
        channel_type: &str,
        scope: Option<&str>,
        identifier: Option<&str>,
    ) -> Option<(String, String)> {
        // Check cache first
        let cache_key = format!(
            "{}:{}:{}",
            channel_type,
            scope.unwrap_or("_"),
            identifier.unwrap_or("_")
        );

        if let Some(agent_id) = self.session_cache.get(&cache_key) {
            let session_key = Self::session_key(agent_id, channel_type, scope, identifier);
            return Some((agent_id.clone(), session_key));
        }

        // Find matching binding (bindings are sorted by priority)
        for binding in &self.bindings {
            if binding.matches(channel_type, scope, identifier) {
                let session_key =
                    Self::session_key(&binding.agent_id, channel_type, scope, identifier);
                self.session_cache
                    .insert(cache_key, binding.agent_id.clone());
                return Some((binding.agent_id.clone(), session_key));
            }
        }

        None
    }

    /// Clear the session cache.
    pub fn clear_cache(&mut self) {
        self.session_cache.clear();
    }

    /// Get all registered bindings.
    #[must_use]
    pub fn bindings(&self) -> &[ChannelBinding] {
        &self.bindings
    }

    /// Get bindings for a specific agent.
    #[must_use]
    pub fn agent_bindings(&self, agent_id: &str) -> Vec<&ChannelBinding> {
        self.bindings
            .iter()
            .filter(|b| b.agent_id == agent_id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_binding_basic() {
        let binding = ChannelBinding::new("agent1", "cli");
        assert_eq!(binding.agent_id, "agent1");
        assert_eq!(binding.channel_type, "cli");
        assert!(binding.matches("cli", None, None));
        assert!(!binding.matches("discord", None, None));
    }

    #[test]
    fn test_channel_binding_with_scope() {
        let binding = ChannelBinding::new("agent1", "discord").with_scope("dm");

        assert!(binding.matches("discord", Some("dm"), None));
        assert!(!binding.matches("discord", Some("guild"), None));
        assert!(!binding.matches("discord", None, None));
    }

    #[test]
    fn test_channel_binding_with_identifier() {
        let binding = ChannelBinding::new("agent1", "discord")
            .with_scope("guild")
            .with_identifier("123456");

        assert!(binding.matches("discord", Some("guild"), Some("123456")));
        assert!(!binding.matches("discord", Some("guild"), Some("999999")));
    }

    #[test]
    fn test_channel_binding_wildcard() {
        let binding = ChannelBinding::new("agent1", "discord")
            .with_scope("*")
            .with_identifier("*");

        assert!(binding.matches("discord", Some("dm"), Some("any")));
        assert!(binding.matches("discord", Some("guild"), Some("other")));
    }

    #[test]
    fn test_session_key() {
        let key = MessageRouter::session_key("agent1", "cli", None, None);
        assert_eq!(key, "agent:agent1:cli:_:_");

        let key = MessageRouter::session_key("agent1", "discord", Some("guild"), Some("123"));
        assert_eq!(key, "agent:agent1:discord:guild:123");
    }

    #[test]
    fn test_router_basic() {
        let mut router = MessageRouter::new();
        router.register(ChannelBinding::new("agent1", "cli"));

        let result = router.route("cli", None, None);
        assert!(result.is_some());
        let (agent_id, session_key) = result.unwrap();
        assert_eq!(agent_id, "agent1");
        assert_eq!(session_key, "agent:agent1:cli:_:_");
    }

    #[test]
    fn test_router_priority() {
        let mut router = MessageRouter::new();
        router.register(ChannelBinding::new("fallback", "discord").with_priority(0));
        router.register(
            ChannelBinding::new("specific", "discord")
                .with_scope("dm")
                .with_priority(10),
        );

        // Should match specific binding due to higher priority
        let result = router.route("discord", Some("dm"), None);
        assert_eq!(result.unwrap().0, "specific");

        // Should fall back to general binding
        let result = router.route("discord", Some("guild"), None);
        assert_eq!(result.unwrap().0, "fallback");
    }

    #[test]
    fn test_router_cache() {
        let mut router = MessageRouter::new();
        router.register(ChannelBinding::new("agent1", "cli"));

        // First call populates cache
        let _ = router.route("cli", None, None);

        // Remove binding
        router.unregister("agent1");

        // Cache should also be cleared
        let result = router.route("cli", None, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_router_no_match() {
        let mut router = MessageRouter::new();
        let result = router.route("unknown", None, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_agent_bindings() {
        let mut router = MessageRouter::new();
        router.register(ChannelBinding::new("agent1", "cli"));
        router.register(ChannelBinding::new("agent1", "discord"));
        router.register(ChannelBinding::new("agent2", "web"));

        let bindings = router.agent_bindings("agent1");
        assert_eq!(bindings.len(), 2);
    }
}
