# astrid-approval

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The single choke point between "the agent wants to" and "the agent gets to."**

In the OS model, this crate is the kernel's security monitor. Every sensitive action, whether it originates from a WASM capsule, an MCP tool call, or the orchestrator itself, flows through `SecurityInterceptor` before it executes. Four independent layers vote. All must agree. One "no" kills the action. Every decision, allowed or denied, hits the audit trail before the caller gets an answer. If the audit write fails, the action is blocked. Fail-closed, not fail-open.

## The four layers

```text
Action arrives
      |
 [1. Policy]       Hard blocks. sudo is always denied. /etc/** is always denied.
      |             Admin-controlled deny lists, path globs, host lists, argument limits.
      |             Cannot be overridden by tokens, allowances, or approvals.
      |
 [2. Capability]   Does a valid ed25519 capability token cover this action?
      |             Checked via astrid-capabilities. Skips approval if found.
      |
 [3. Budget]       Is the session within its spending limit?
      |             Per-action and per-session USD limits, enforced atomically.
      |             Dual budget: session AND workspace must both allow.
      |             Reservation-based: cost is held during approval, refunded on denial.
      |
 [4. Approval]     No token and no allowance? Ask the human.
      |             Allow Once / Allow Session / Allow Workspace / Allow Always / Deny.
      |             "Allow Always" mints a signed capability token for next time.
      |             "Allow Session" creates a scoped allowance that auto-matches future calls.
      |             Human unavailable? Action queues as DeferredResolution. Does not silently skip.
```

Intersection semantics. Not a pipeline where later stages can override earlier ones. Policy blocks are absolute.

## Key design decisions

**Atomic budget reservation.** `BudgetTracker::check_and_reserve` holds a single write lock for check + debit. Two concurrent callers cannot both pass the check and then both debit. Cancelled futures refund automatically via drop guard. `WorkspaceBudgetTracker` applies the same guarantee across sessions. On dual-budget denial, the workspace reservation rolls back before the error propagates.

**Nine `AllowancePattern` variants.** `ExactTool`, `ServerTools`, `FilePattern`, `NetworkHost`, `CommandPattern`, `WorkspaceRelative`, `Custom`, `CapsuleCapability`, `CapsuleWildcard`. All use `globset` for matching. Path traversal and shell operators are rejected at the pattern-match layer.

**14 `SensitiveAction` variants.** Every gated operation has a dedicated enum variant with typed context: `FileRead`, `FileDelete`, `FileWriteOutsideSandbox`, `ExecuteCommand`, `NetworkRequest`, `TransmitData`, `FinancialTransaction`, `AccessControlChange`, `CapabilityGrant`, `McpToolCall`, `CapsuleExecution`, `CapsuleHttpRequest`, `CapsuleFileAccess`, `CapsuleNetBind`. Risk levels are assigned per variant (Critical for financial, High for deletes, Medium for reads).

**Fail-closed audit.** Every allow, deny, and defer writes an `AuditLog` entry. Write fails? `ApprovalError::AuditFailed`. The action never executes.

## Usage

```toml
[dependencies]
astrid-approval = { workspace = true }
```

```rust
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
    None,
);

let action = SensitiveAction::FileDelete {
    path: "/home/user/report.txt".to_string(),
};
match interceptor.intercept(&action, "removing stale report", None).await {
    Ok(result) => { /* result.proof, result.audit_id, result.budget_warning */ }
    Err(e) => eprintln!("blocked: {e}"),
}
```

Frontends implement the `ApprovalHandler` trait (`request_approval`, `is_available`) to present approval prompts. The kernel registers the handler on the `ApprovalManager`.

## Development

```bash
cargo test -p astrid-approval
```

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
