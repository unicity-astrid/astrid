# astrid-approval

[![Crates.io](https://img.shields.io/crates/v/astrid-approval)](https://crates.io/crates/astrid-approval)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

> "Math, not hope." The deterministic security, budget, and enforcement engine for Astralis.

In the era of autonomous agents, prompt engineering is not a security boundary. `astrid-approval` provides a rigorous, intersection-based security model that evaluates every sensitive action an agent attempts to make. From MCP tool calls to filesystem writes and plugin executions, if an action is not explicitly permitted by a cryptographic capability, within budget, and allowed by the static workspace policy, it does not happen. Period.

## Core Features

*   **Unified Security Interceptor**: A single architectural choke point for all agent actions. Every request flows through a strict intersection check combining policies, budgets, capabilities, and dynamic human approval.
*   **Dual-Layer Budget Tracking**: Precise, race-condition-free cost control tracking both isolated Session Budgets and cumulative Workspace Budgets via an atomic check-and-reserve pattern.
*   **Cryptographic Allowances**: Fine-grained, pattern-based grants that give agents persistent or session-scoped access to specific MCP tools, file globs, network hosts, or isolated plugin capabilities.
*   **Hard Boundaries**: Admin-configured static rules that supersede any LLM prompt or capability. Block dangerous commands, deny sensitive paths, or blacklist domains.
*   **Deferred Resolutions**: When a human is required but offline, the system safely queues the action, allowing the agent to gracefully switch contexts rather than hanging indefinitely.

## Architecture: Intersection Semantics

The `SecurityInterceptor` is the strictly defined structural entrypoint of this crate. It evaluates actions using strict **intersection semantics**—multiple independent layers must agree before the runtime executes an action.

1.  **Policy Check (Hard Boundaries)**: Is the tool explicitly blocked? Does it target a denied path, exceed argument size limits, or target a blocked plugin? If so, immediately deny.
2.  **Capability Check (Grants)**: Does the agent possess a cryptographically valid capability or allowance for this exact action?
3.  **Budget Check (Financials)**: Is there enough remaining session and workspace budget? The `BudgetTracker` atomically checks and reserves the estimated cost to prevent time-of-check to time-of-use (TOCTOU) vulnerabilities.
4.  **Risk Assessment & Approval**: If the action inherently requires approval and lacks a capability, the interceptor pauses execution and delegates to the `ApprovalManager`.
5.  **Audit Trail**: Every decision—whether allowed, denied, or deferred—is immutably written to the workspace audit log.

## Quick Start

`astrid-approval` is designed to be embedded directly into the Astralis execution pipeline.

```rust
use astrid_approval::{SecurityInterceptor, SecurityPolicy};
use astrid_approval::budget::BudgetTracker;
use astrid_approval::action::SensitiveAction;
use std::sync::Arc;

// 1. Define hard boundaries (defaults block traversal and dangerous paths)
let policy = SecurityPolicy::default();

// 2. Initialize the interceptor (handled by the Astralis runtime)
let interceptor = SecurityInterceptor::new(
    capability_store,
    approval_manager,
    policy,
    budget_tracker,
    audit_log,
    runtime_key,
    session_id,
    allowance_store,
    Some(workspace_root),
    Some(workspace_budget),
);

// 3. Intercept sensitive actions before they hit the real system
let action = SensitiveAction::FileDelete { 
    path: "/home/user/important.txt".to_string() 
};

// Automatically evaluates policy, deducts budget, or requests human approval
match interceptor.intercept(&action, "Cleaning up temp files", None).await {
    Ok(result) => println!("Action permitted. Proof: {:?}", result.proof),
    Err(e) => eprintln!("Action blocked: {}", e),
}
```

## Budget Enforcement

The `BudgetTracker` enforces both a maximum spend per session and a maximum spend per individual action. It utilizes an atomic "check and reserve" pattern under a single write lock to prevent race conditions where concurrent actions might bypass limits.

```rust
use astrid_approval::budget::{BudgetConfig, BudgetTracker};

// Max $10.00 for the session, max $5.00 per individual action.
// Automatically warns the user when 80% of the session budget is consumed.
let config = BudgetConfig::new(10.0, 5.0).with_warn_at_percent(80);
let tracker = BudgetTracker::new(config);

// Costs are atomically reserved, and can be refunded if the action fails
let result = tracker.check_and_reserve(1.50);
assert!(result.is_allowed());
```

## Allowance Patterns & Discovery

In accordance with Astralis architectural guidelines, internal abstractions use closed-set Enums (such as `SensitiveAction` and `AllowancePattern`). This ensures exhaustive `match` statements across the workspace flag integration points at compile time whenever a new capability is introduced.

When users approve an action, the system generates a signed `Allowance` using an `AllowancePattern` to prevent repetitive prompting:

*   `ExactTool`: Precise access to an MCP tool.
*   `FilePattern`: Glob-based file access (e.g., `src/**/*.rs` with `Write` permission).
*   `NetworkHost`: Outbound HTTP/TCP access to a domain.
*   `WorkspaceRelative`: Safely scopes file and tool allowances to the bounds of the current workspace directory, preventing directory traversal attacks.
*   `CapsuleCapability`: Isolated execution grants for specific plugin architectures.

## The Approval Manager

When an action requires a human in the loop, the `ApprovalManager` routes an `ApprovalRequest` to the active frontend via the `ApprovalHandler` trait. Frontends implement this trait to render the request. If the frontend is disconnected or the request times out, the manager seamlessly queues it as a `DeferredResolution`.

## Development

This crate acts as the core security dependency of Astralis. Run tests via the workspace:

```bash
cargo test -p astrid-approval
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.