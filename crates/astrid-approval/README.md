# astrid-approval

[![Crates.io](https://img.shields.io/crates/v/astrid-approval)](https://crates.io/crates/astrid-approval)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Human-in-the-loop approval, budget enforcement, and security policy for the Astrid agent runtime.

Every sensitive action an agent attempts passes through a single architectural choke point - `SecurityInterceptor` - which applies intersection semantics across four independent layers: hard-boundary policy, cryptographic capability tokens, session/workspace budget limits, and dynamic human approval. If any layer denies the action, it does not execute. Every decision is written to the audit trail before the caller receives an answer (fail-closed).

## Core Features

- **Intersection-semantics interceptor**: Policy, capability, budget, and approval must all agree. One `No` stops the action regardless of what the others say.
- **`SecurityPolicy` with hard boundaries**: Admin-configured rules that block dangerous commands (`sudo`, `rm -rf /`, `mkfs`), deny sensitive paths (`/etc/**`, `/proc/**`), enforce argument size limits, and maintain a blocked-capsules set. Checked before any dynamic logic runs.
- **Atomic budget tracking**: `BudgetTracker` enforces per-action and per-session USD limits via `check_and_reserve` - a single write-lock operation that eliminates TOCTOU races. `WorkspaceBudgetTracker` applies the same guarantee across sessions for cumulative workspace spend. Budget reservations are dropped (refunded) automatically if the future is cancelled.
- **Signed allowances**: When a user approves an action, the approval can create a cryptographically signed `Allowance` scoped to the session, workspace, or permanently. Subsequent identical actions match the stored allowance without re-prompting.
- **`AllowancePattern` matching**: Eight pattern variants - `ExactTool`, `ServerTools`, `FilePattern`, `NetworkHost`, `CommandPattern`, `WorkspaceRelative`, `CapsuleCapability`, `CapsuleWildcard` - cover every `SensitiveAction` variant. Glob matching via `globset`. Path traversal sequences and shell operators are rejected at the pattern-match layer.
- **`ApprovalHandler` trait**: Frontends (CLI, Discord, Web) implement this trait to present requests to the user. The manager routes requests to the registered handler with a configurable timeout (default 5 minutes).
- **Deferred resolution queue**: When no handler is registered, the handler reports unavailable, or the request times out, the action is queued as a `DeferredResolution` with priority and fallback behavior rather than blocking the agent indefinitely.
- **Fail-closed audit**: Every allow, deny, and defer writes an `AuditLog` entry. If the write fails, the action is blocked and `ApprovalError::AuditFailed` is returned.

## Quick Start

```toml
[dependencies]
astrid-approval = { workspace = true }
```

```rust
use astrid_approval::{SecurityInterceptor, SecurityPolicy, SensitiveAction};
use astrid_approval::budget::{BudgetConfig, BudgetTracker};
use astrid_approval::manager::{ApprovalManager, ApprovalHandler};
use astrid_approval::allowance::AllowanceStore;
use astrid_approval::deferred::DeferredResolutionStore;
use std::sync::Arc;

// Hard-boundary defaults: blocks sudo, rm -rf /, /etc/**, /proc/**, 1 MB arg limit,
// requires approval for deletes and network.
let policy = SecurityPolicy::default();

let budget = Arc::new(BudgetTracker::new(BudgetConfig::new(100.0, 10.0)));
let allowance_store = Arc::new(AllowanceStore::new());
let approval_manager = Arc::new(ApprovalManager::new(
    Arc::clone(&allowance_store),
    Arc::new(DeferredResolutionStore::new()),
));

// Register a frontend handler - the CLI, Discord bot, or web UI implements ApprovalHandler.
approval_manager.register_handler(Arc::new(my_cli_handler)).await;

let interceptor = SecurityInterceptor::new(
    capability_store,
    approval_manager,
    policy,
    budget,
    audit_log,
    runtime_key,
    session_id,
    allowance_store,
    Some(workspace_root),
    None, // optional workspace budget tracker
);

// Every sensitive action goes through intercept() before execution.
let action = SensitiveAction::FileDelete {
    path: "/home/user/report.txt".to_string(),
};

match interceptor.intercept(&action, "removing stale report", None).await {
    Ok(result) => {
        // result.proof says how it was authorized (capability, allowance, user approval, etc.)
        // result.audit_id is the immutable audit trail entry
        // result.budget_warning is Some(...) if spend is approaching the session limit
    }
    Err(e) => eprintln!("blocked: {e}"),
}
```

## API Reference

### Key Types

- **`SensitiveAction`** - Enum of every action category that can require approval: `FileRead`, `FileDelete`, `FileWriteOutsideSandbox`, `ExecuteCommand`, `NetworkRequest`, `TransmitData`, `FinancialTransaction`, `AccessControlChange`, `CapabilityGrant`, `McpToolCall`, `CapsuleExecution`, `CapsuleHttpRequest`, `CapsuleFileAccess`, `CapsuleNetBind`. Each variant carries the context needed for an informed allow/deny decision.
- **`SecurityPolicy`** - Serializable struct with `blocked_tools`, `approval_required_tools`, `allowed_paths`, `denied_paths`, `allowed_hosts`, `denied_hosts`, `max_argument_size`, and `blocked_capsules`. Use `SecurityPolicy::default()` for sensible production defaults or `SecurityPolicy::permissive()` for testing.
- **`SecurityInterceptor`** - Main entry point. Constructed once and shared via `Arc`. Call `intercept(&action, context, estimated_cost)` for every sensitive operation.
- **`ApprovalRequest` / `ApprovalDecision` / `ApprovalResponse`** - The approval flow types. `ApprovalDecision` has five approval variants: `Approve` (once), `ApproveSession`, `ApproveWorkspace`, `ApproveAlways` (mints a capability token), `ApproveWithAllowance`.
- **`RiskAssessment`** - Carries `RiskLevel` (from `astrid-core`), a reason string, and optional mitigations. Built automatically from `SensitiveAction::default_risk_level()`.
- **`BudgetTracker`** / **`BudgetConfig`** - Session-scoped budget with `check_budget`, `check_and_reserve`, `record_cost`, `refund_cost`, `snapshot`, and `restore`. Thread-safe via internal `RwLock`.
- **`WorkspaceBudgetTracker`** - Cumulative cross-session budget tracker. Optional cap (`max_usd: None` records spend for reporting without blocking).
- **`Allowance`** / **`AllowancePattern`** / **`AllowanceStore`** - Pre-approved action grants. Allowances carry a `Signature` proving they were legitimately created, optional expiry, and optional use limits.
- **`ApprovalHandler`** (trait) - Frontend interface. Implement `request_approval` and `is_available`.
- **`ApprovalManager`** - Orchestrates allowance lookup, handler dispatch, timeout, and deferred queuing.
- **`DeferredResolutionStore`** - In-memory queue for actions that could not be immediately resolved. Supports optional `ScopedKvStore` persistence with 24-hour stale-item eviction on load.
- **`ApprovalError`** - Error enum: `Denied`, `Timeout`, `Deferred`, `PolicyBlocked`, `Storage`, `Internal`, `AuditFailed`.
- **`InterceptResult`** / **`InterceptProof`** - Successful intercept outcome carrying the authorization proof and audit entry ID.

## Development

```bash
cargo test -p astrid-approval
```

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
