use std::time::Duration;
use super::{
    DemoScenario, DemoStep, ToolRisk, ApprovalChoice, View, 
    NexusCategoryDemo, SidebarState, TaskStatus, FileStatus,
    AgentStatusDemo, AuditOutcomeDemo, HealthStatusDemo, ThreatLevelDemo
};

#[allow(clippy::too_many_lines)]
pub(super) fn build() -> DemoScenario {
    DemoScenario {
        name: "multi-agent-ops".to_string(),
        description: "Multi-agent: 3 agents, sub-agents, approvals, audit chain".to_string(),
        steps: vec![
            // ═══════════════════════════════════════════════════════════
            // ACT 1: BOOT + SPAWN AGENTS
            // ═══════════════════════════════════════════════════════════
            DemoStep::Narrate("Act 1: Multi-Agent Boot".to_string()),
            DemoStep::BootSequence {
                cinematic: true,
                checks: vec![
                    ("Runtime initialized".to_string(), true),
                    ("Gateway connected".to_string(), true),
                    ("Agent pool: 3 slots".to_string(), true),
                    ("MCP servers: 4 online".to_string(), true),
                ],
            },
            DemoStep::Pause(Duration::from_millis(800)),
            DemoStep::Clear,
            // Spawn 3 agents
            DemoStep::SpawnAgent {
                name: "alpha".to_string(),
                model: "claude-opus-4.6".to_string(),
            },
            DemoStep::Pause(Duration::from_millis(300)),
            DemoStep::SpawnAgent {
                name: "beta".to_string(),
                model: "claude-sonnet-4.5".to_string(),
            },
            DemoStep::Pause(Duration::from_millis(300)),
            DemoStep::SpawnAgent {
                name: "gamma".to_string(),
                model: "claude-haiku-4.5".to_string(),
            },
            DemoStep::Pause(Duration::from_millis(500)),
            // ═══════════════════════════════════════════════════════════
            // ACT 2: COMMAND CENTER
            // ═══════════════════════════════════════════════════════════
            DemoStep::Narrate("Act 2: Command Center".to_string()),
            DemoStep::SwitchView(View::Command),
            DemoStep::Pause(Duration::from_millis(1000)),
            // Agents start working
            DemoStep::SetAgentStatus {
                agent: "alpha".to_string(),
                status: AgentStatusDemo::Busy,
            },
            DemoStep::Pause(Duration::from_millis(400)),
            DemoStep::SetAgentStatus {
                agent: "beta".to_string(),
                status: AgentStatusDemo::Ready,
            },
            DemoStep::Pause(Duration::from_millis(400)),
            DemoStep::SetAgentStatus {
                agent: "gamma".to_string(),
                status: AgentStatusDemo::Busy,
            },
            DemoStep::Pause(Duration::from_millis(600)),
            // Events flow in
            DemoStep::AddEventRecord {
                agent: "alpha".to_string(),
                event_type: "McpToolCalled".to_string(),
                detail: "read_file(src/auth.rs)".to_string(),
            },
            DemoStep::Pause(Duration::from_millis(300)),
            DemoStep::AddEventRecord {
                agent: "gamma".to_string(),
                event_type: "McpToolCalled".to_string(),
                detail: "search_code(\"password\")".to_string(),
            },
            DemoStep::Pause(Duration::from_millis(300)),
            DemoStep::AddEventRecord {
                agent: "alpha".to_string(),
                event_type: "CapabilityGranted".to_string(),
                detail: "mcp://fs:read_file session".to_string(),
            },
            DemoStep::Pause(Duration::from_millis(1500)),
            // ═══════════════════════════════════════════════════════════
            // ACT 3: TOPOLOGY - Sub-agent delegation
            // ═══════════════════════════════════════════════════════════
            DemoStep::Narrate("Act 3: Topology - Sub-Agent Delegation".to_string()),
            DemoStep::SwitchView(View::Topology),
            DemoStep::Pause(Duration::from_millis(800)),
            // Alpha delegates sub-tasks
            DemoStep::SpawnSubAgent {
                parent_agent: "alpha".to_string(),
                task: "Analyze auth patterns".to_string(),
            },
            DemoStep::Pause(Duration::from_millis(500)),
            DemoStep::SpawnSubAgent {
                parent_agent: "alpha".to_string(),
                task: "Read config files".to_string(),
            },
            DemoStep::Pause(Duration::from_millis(500)),
            // Gamma delegates too
            DemoStep::SpawnSubAgent {
                parent_agent: "gamma".to_string(),
                task: "Deploy to staging".to_string(),
            },
            DemoStep::Pause(Duration::from_millis(1000)),
            // One completes
            DemoStep::CompleteSubAgent {
                id: "sub-002".to_string(),
                success: true,
            },
            DemoStep::Pause(Duration::from_millis(800)),
            // One fails
            DemoStep::CompleteSubAgent {
                id: "sub-003".to_string(),
                success: false,
            },
            DemoStep::Pause(Duration::from_millis(800)),
            DemoStep::SetAgentStatus {
                agent: "gamma".to_string(),
                status: AgentStatusDemo::Error,
            },
            DemoStep::Pause(Duration::from_millis(1200)),
            // ═══════════════════════════════════════════════════════════
            // ACT 4: SHIELD - Security Dashboard
            // ═══════════════════════════════════════════════════════════
            DemoStep::Narrate("Act 4: Shield - Security Dashboard".to_string()),
            DemoStep::SwitchView(View::Shield),
            DemoStep::Pause(Duration::from_millis(800)),
            // Grant capabilities
            DemoStep::GrantCapability {
                agent: "alpha".to_string(),
                resource: "mcp://fs:read_*".to_string(),
                scope: "session".to_string(),
                ttl_secs: Some(7200),
            },
            DemoStep::Pause(Duration::from_millis(400)),
            DemoStep::GrantCapability {
                agent: "alpha".to_string(),
                resource: "mcp://fs:write_*".to_string(),
                scope: "persistent".to_string(),
                ttl_secs: None,
            },
            DemoStep::Pause(Duration::from_millis(400)),
            // Approval requests
            DemoStep::AddShieldApproval {
                agent: "alpha".to_string(),
                tool: "delete_files(**/*.tmp)".to_string(),
                risk: ToolRisk::High,
            },
            DemoStep::Pause(Duration::from_millis(400)),
            DemoStep::AddShieldApproval {
                agent: "gamma".to_string(),
                tool: "execute_cmd(deploy.sh)".to_string(),
                risk: ToolRisk::High,
            },
            DemoStep::Pause(Duration::from_millis(800)),
            // Security violation
            DemoStep::SecurityViolation {
                agent: "gamma".to_string(),
                detail: "Workspace escape attempt: /etc/passwd".to_string(),
            },
            DemoStep::Pause(Duration::from_millis(600)),
            DemoStep::SetThreatLevel(ThreatLevelDemo::Elevated),
            DemoStep::Pause(Duration::from_millis(1500)),
            // ═══════════════════════════════════════════════════════════
            // ACT 5: PULSE - Health & Budget
            // ═══════════════════════════════════════════════════════════
            DemoStep::Narrate("Act 5: Pulse - Health & Budget".to_string()),
            DemoStep::SwitchView(View::Pulse),
            DemoStep::Pause(Duration::from_millis(800)),
            // Set health checks
            DemoStep::SetHealth {
                component: "agent_manager".to_string(),
                status: HealthStatusDemo::Ok,
            },
            DemoStep::SetHealth {
                component: "mcp_pool".to_string(),
                status: HealthStatusDemo::Ok,
            },
            DemoStep::SetHealth {
                component: "approval_queue".to_string(),
                status: HealthStatusDemo::Ok,
            },
            DemoStep::SetHealth {
                component: "audit_log".to_string(),
                status: HealthStatusDemo::Degraded,
            },
            DemoStep::Pause(Duration::from_millis(400)),
            // Set budget data
            DemoStep::SetBudget {
                agent: "alpha".to_string(),
                spent: 1.43,
            },
            DemoStep::SetBudget {
                agent: "beta".to_string(),
                spent: 0.00,
            },
            DemoStep::SetBudget {
                agent: "gamma".to_string(),
                spent: 0.71,
            },
            DemoStep::Pause(Duration::from_millis(1500)),
            // ═══════════════════════════════════════════════════════════
            // ACT 6: CHAIN - Audit Trail
            // ═══════════════════════════════════════════════════════════
            DemoStep::Narrate("Act 6: Chain - Audit Trail".to_string()),
            DemoStep::SwitchView(View::Chain),
            DemoStep::Pause(Duration::from_millis(800)),
            // Add audit entries
            DemoStep::AddAuditEntry {
                agent: "alpha".to_string(),
                action: "SessionStart".to_string(),
                outcome: AuditOutcomeDemo::Success,
            },
            DemoStep::Pause(Duration::from_millis(200)),
            DemoStep::AddAuditEntry {
                agent: "alpha".to_string(),
                action: "McpToolCall".to_string(),
                outcome: AuditOutcomeDemo::Success,
            },
            DemoStep::Pause(Duration::from_millis(200)),
            DemoStep::AddAuditEntry {
                agent: "alpha".to_string(),
                action: "CapabilityGrant".to_string(),
                outcome: AuditOutcomeDemo::Success,
            },
            DemoStep::Pause(Duration::from_millis(200)),
            DemoStep::AddAuditEntry {
                agent: "gamma".to_string(),
                action: "McpToolCall".to_string(),
                outcome: AuditOutcomeDemo::Success,
            },
            DemoStep::Pause(Duration::from_millis(200)),
            DemoStep::AddAuditEntry {
                agent: "gamma".to_string(),
                action: "SecurityViolation".to_string(),
                outcome: AuditOutcomeDemo::Violation,
            },
            DemoStep::Pause(Duration::from_millis(200)),
            DemoStep::AddAuditEntry {
                agent: "alpha".to_string(),
                action: "ApprovalRequested".to_string(),
                outcome: AuditOutcomeDemo::Success,
            },
            DemoStep::Pause(Duration::from_millis(1500)),
            // ═══════════════════════════════════════════════════════════
            // FINALE
            // ═══════════════════════════════════════════════════════════
            DemoStep::Narrate("✧ Multi-Agent Demo Complete ✧".to_string()),
            DemoStep::Pause(Duration::from_millis(750)),
            DemoStep::SwitchView(View::Command),
            DemoStep::Pause(Duration::from_millis(500)),
            DemoStep::OrbitStatus("* Demo complete — all agent views demonstrated".to_string()),
            DemoStep::Pause(Duration::from_secs(3)),
            DemoStep::SystemMessage(String::new()),
            DemoStep::SystemMessage("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string()),
            DemoStep::SystemMessage("  ✧ A S T R A L I S ✧".to_string()),
            DemoStep::SystemMessage("  Multi-Agent Demo:".to_string()),
            DemoStep::SystemMessage("    * Command: Agent grid overhead".to_string()),
            DemoStep::SystemMessage("    * Topology: Sub-agent tree".to_string()),
            DemoStep::SystemMessage("    * Shield: Security dashboard".to_string()),
            DemoStep::SystemMessage("    * Pulse: Health & budget".to_string()),
            DemoStep::SystemMessage("    * Chain: Audit trail".to_string()),
            DemoStep::SystemMessage("    * 3 agents, sub-delegation".to_string()),
            DemoStep::SystemMessage("    * Capability tokens, threat levels".to_string()),
            DemoStep::SystemMessage("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string()),
            DemoStep::Pause(Duration::from_millis(7500)),
        ],
    }
}
