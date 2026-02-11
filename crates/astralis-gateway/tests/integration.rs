//! Integration tests for the gateway runtime.

use astralis_gateway::config::GatewayConfig;
use astralis_gateway::runtime::{GatewayRuntime, RuntimeState};
use astralis_gateway::state::PersistedState;
use tempfile::TempDir;

/// Test full gateway lifecycle: create -> start -> health check -> shutdown.
#[tokio::test]
async fn test_gateway_lifecycle() {
    let config = GatewayConfig::default();
    let mut runtime = GatewayRuntime::new(config).unwrap();

    // Initially in Initializing state
    assert_eq!(runtime.state().await, RuntimeState::Initializing);

    // Start the runtime
    runtime.start().await.unwrap();
    assert_eq!(runtime.state().await, RuntimeState::Running);

    // Health should be OK (or degraded with no agents, which is fine)
    let health = runtime.health().await;
    assert!(health.is_healthy() || health.state == astralis_gateway::health::HealthState::Degraded);

    // Shutdown
    runtime.shutdown().await.unwrap();
    assert_eq!(runtime.state().await, RuntimeState::Stopped);
}

/// Test health check returns proper status.
#[tokio::test]
async fn test_gateway_health() {
    let config = GatewayConfig::default();
    let runtime = GatewayRuntime::new(config).unwrap();

    let health = runtime.health().await;

    // Should have version info
    assert!(!health.version.is_empty());

    // Should have component checks
    assert!(!health.checks.is_empty());
}

/// Test state persistence with proper file permissions.
#[tokio::test]
async fn test_state_persistence() {
    let temp = TempDir::new().unwrap();
    let state_path = temp.path().join("state.json");

    // Create and save state
    let mut state = PersistedState::new();
    state.set_agent(
        "test-agent",
        astralis_gateway::state::AgentState {
            name: "test-agent".into(),
            session_id: Some("session-123".into()),
            last_activity: Some(chrono::Utc::now()),
            request_count: 42,
            error_count: 3,
            metadata: Default::default(),
        },
    );

    state.save(&state_path).unwrap();

    // Verify file exists
    assert!(state_path.exists());

    // On Unix, verify permissions are 0600
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::metadata(&state_path).unwrap().permissions();
        assert_eq!(permissions.mode() & 0o777, 0o600);
    }

    // Load and verify
    let loaded = PersistedState::load(&state_path).unwrap();
    let agent = loaded.agent("test-agent").unwrap();
    assert_eq!(agent.request_count, 42);
    assert_eq!(agent.error_count, 3);
}

/// Test pending approvals management.
#[tokio::test]
async fn test_pending_approvals() {
    let mut state = PersistedState::new();

    // Add approval
    state.add_pending_approval(astralis_gateway::state::PendingApproval {
        id: "approval-1".into(),
        agent_name: "test-agent".into(),
        session_id: "session-1".into(),
        approval_type: "tool_call".into(),
        description: "Execute command".into(),
        requested_at: chrono::Utc::now(),
        expires_at: None,
        risk_level: "high".into(),
        tool_name: Some("execute".into()),
        context: Default::default(),
    });

    assert_eq!(state.pending_approvals.len(), 1);

    // Get for agent
    let approvals = state.agent_pending_approvals("test-agent");
    assert_eq!(approvals.len(), 1);

    // Remove
    let removed = state.remove_pending_approval("approval-1");
    assert!(removed.is_some());
    assert!(state.pending_approvals.is_empty());
}

/// Test task queue priority ordering.
#[tokio::test]
async fn test_task_queue() {
    let mut state = PersistedState::new();

    // Queue tasks with different priorities
    state.queue_task(astralis_gateway::state::QueuedTask {
        id: "task-low".into(),
        agent_name: "agent".into(),
        task_type: "message".into(),
        payload: serde_json::json!({"priority": "low"}),
        queued_at: chrono::Utc::now(),
        priority: 1,
        retry_count: 0,
        last_error: None,
    });

    state.queue_task(astralis_gateway::state::QueuedTask {
        id: "task-high".into(),
        agent_name: "agent".into(),
        task_type: "message".into(),
        payload: serde_json::json!({"priority": "high"}),
        queued_at: chrono::Utc::now(),
        priority: 10,
        retry_count: 0,
        last_error: None,
    });

    // Should get high priority first
    let task = state.pop_task("agent").unwrap();
    assert_eq!(task.id, "task-high");

    let task = state.pop_task("agent").unwrap();
    assert_eq!(task.id, "task-low");
}

/// Test config hot-reload functionality.
#[tokio::test]
async fn test_config_hot_reload() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("gateway.toml");

    // Initial config
    std::fs::write(
        &config_path,
        r#"
        [gateway]
        hot_reload = true
        health_interval_secs = 30

        [timeouts]
        request_secs = 60
        "#,
    )
    .unwrap();

    let config = GatewayConfig::load(&config_path).unwrap();
    let runtime = GatewayRuntime::with_config_path(config, Some(config_path.clone())).unwrap();

    // Verify initial
    {
        let config = runtime.config().read().await;
        assert_eq!(config.timeouts.request_secs, 60);
    }

    // Update config
    std::fs::write(
        &config_path,
        r#"
        [gateway]
        hot_reload = true
        health_interval_secs = 30

        [timeouts]
        request_secs = 120
        "#,
    )
    .unwrap();

    // Reload
    runtime.reload_config().await.unwrap();

    // Verify updated
    {
        let config = runtime.config().read().await;
        assert_eq!(config.timeouts.request_secs, 120);
    }
}

/// Test subagent state tracking.
#[tokio::test]
async fn test_subagent_state() {
    let mut state = PersistedState::new();

    state.subagents.insert(
        "sub-1".into(),
        astralis_gateway::state::SubAgentState {
            id: "sub-1".into(),
            parent_id: None,
            task: "Research topic X".into(),
            depth: 1,
            status: "running".into(),
            started_at: chrono::Utc::now(),
            completed_at: None,
        },
    );

    state.subagents.insert(
        "sub-2".into(),
        astralis_gateway::state::SubAgentState {
            id: "sub-2".into(),
            parent_id: Some("sub-1".into()),
            task: "Subtask of sub-1".into(),
            depth: 2,
            status: "running".into(),
            started_at: chrono::Utc::now(),
            completed_at: None,
        },
    );

    assert_eq!(state.subagents.len(), 2);

    // Save and load
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("state.json");
    state.save(&path).unwrap();

    let loaded = PersistedState::load(&path).unwrap();
    assert_eq!(loaded.subagents.len(), 2);
    assert_eq!(
        loaded.subagents.get("sub-2").unwrap().parent_id,
        Some("sub-1".into())
    );
}
