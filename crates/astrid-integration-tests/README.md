# astrid-integration-tests

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

The proving ground for the Astrid workspace, validating end-to-end security, capability isolation, and cross-component interactions.

`astrid-integration-tests` is a private, `publish = false` crate containing no library code. Its sole purpose is to stitch together the components of the Astrid runtime - capabilities, capsules, MCP bridges, the approval matrix, and the audit log - and verify they behave cohesively under hostile or complex conditions. Every test runs against the actual engine: real WebAssembly payloads, real `SecurityInterceptor` pipelines, real `CapabilityStore` and `AllowanceStore` instances. Nothing is mocked.

## Core Features

- **WASM capsule sandbox enforcement**: End-to-end tests for memory allocation traps (64 KB log limit, 10 MB KV limit), IPC payload and subscription limits, VFS path traversal rejection (`../../etc/passwd`), `global://` VFS scheme access control, and legitimate read/write round-trips within the workspace boundary.
- **HTTP security gate**: Verifies that capsules can only reach hosts declared in their manifest's `net` capability array. Requests to undeclared hosts return a "security denied / not declared in manifest" error. CRLF header injection attempts are rejected by the reqwest layer before any network access occurs.
- **MCP host-process capability checking**: Tests that `CapsuleLoader` refuses to load a capsule whose `mcp_servers` entry requests a binary (`python3`) not listed in the manifest's `host_process` capability array.
- **Approval and allowance flow**: Deterministic `ApprovalHandler` implementations verify the full state machine - "Approve Once", "Approve Session" (auto-approves subsequent identical actions without re-prompting), "Approve Workspace" (survives `clear_session_allowances()`), and "Approve Always" (creates a persistent `CapabilityToken`). Cross-workspace isolation is also verified: an allowance scoped to `/project-a` must not match when the interceptor's `workspace_root` is `/project-b`.
- **Capability token persistence and reuse**: Tests that "Approve Always" creates a `CapabilityToken` stored in the `CapabilityStore`, that subsequent identical actions hit the capability path rather than the approval path, and that capability tokens survive across independent `SecurityInterceptor` instances sharing the same store.
- **Security regression suite**: Seven targeted regression tests covering confirmed past bugs - atomic allowance find-and-consume under 10 concurrent tasks (max-uses:1 must be consumed exactly once), atomic budget check-and-reserve under 20 concurrent tasks (no overspend), workspace budget not bypassed by a valid capability token, allowance creation emitting exactly one audit entry, path traversal detection via `Path::components` (handles edge cases like triple-dot paths), expired allowance cleanup on lookup, and atomic workspace budget check-and-reserve.
- **Capsule lifecycle dispatch**: Tests for install and upgrade hook execution (`astrid_install`, `astrid_upgrade` WASM exports), including graceful skip when the export is absent, hard error on invalid WASM bytes, elicit-request handling during install, and KV write verification after upgrade.
- **Env/config injection**: Verifies that `EnvDef` defaults and KV-injected values are correctly surfaced to WASM tools via the `test-config` endpoint, and that missing keys return `found: false`.

## Architecture

This crate depends on `astrid-test` for harness utilities and `astrid-openclaw` (dev-dependency) for the pre-compiled `test-all-endpoints.wasm` fixture in `tests/fixtures/`. Tests that require the WASM fixture skip gracefully if the file is absent rather than failing, so the suite remains runnable in environments where the fixture has not been built.

The interceptor pipeline assembled in each test instantiates the same types the production runtime uses: `SecurityInterceptor`, `ApprovalManager`, `AllowanceStore`, `CapabilityStore`, `BudgetTracker`, `WorkspaceBudgetTracker`, and `AuditLog`. This means any regression caught here is a regression in production behavior, not a test double.

## Development

Ensure the WASM test fixture is compiled before running tests that exercise the capsule engine:

```bash
./scripts/compile-test-plugin.sh
```

Run the full integration suite:

```bash
cargo test -p astrid-integration-tests
```

Run a specific test file:

```bash
# WASM capability and VFS tests
cargo test -p astrid-integration-tests --test wasm_e2e

# MCP host-process capability check
cargo test -p astrid-integration-tests --test mcp_e2e

# Allowance and approval state machine
cargo test -p astrid-integration-tests --test allowance_flow

# Capability token lifecycle (Allow Always flow)
cargo test -p astrid-integration-tests --test capability_lifecycle

# Security regression suite (atomic budget, race conditions, path traversal)
cargo test -p astrid-integration-tests --test security_regressions

# Capsule install/upgrade lifecycle hooks
cargo test -p astrid-integration-tests --test lifecycle_e2e

# Env/config injection into WASM tools
cargo test -p astrid-integration-tests --test wasm_env_e2e
```

### Test file inventory

| File | What it covers |
|---|---|
| `tests/wasm_e2e.rs` | Capsule sandbox: memory limits, IPC limits, VFS path traversal, `global://` VFS, HTTP security gate, CRLF injection |
| `tests/mcp_e2e.rs` | MCP host-process capability validation on capsule load |
| `tests/allowance_flow.rs` | Session/workspace allowance state machine, cross-workspace isolation |
| `tests/capability_lifecycle.rs` | `ApproveAlways` creates `CapabilityToken`; token reuse across interceptor instances |
| `tests/security_regressions.rs` | Seven atomic/race-condition/audit regressions from Step 8 security review |
| `tests/lifecycle_e2e.rs` | Install/upgrade WASM hook dispatch, elicit handling, KV write verification |
| `tests/wasm_env_e2e.rs` | `EnvDef` default and KV-injected value surfacing to WASM tools |
| `tests/fixtures/` | Pre-compiled `test-all-endpoints.wasm` (built by `scripts/compile-test-plugin.sh`) |
| `src/lib.rs` | Crate entry point; no functional code, enforces workspace lint rules |

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
