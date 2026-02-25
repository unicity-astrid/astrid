# astrid-integration-tests

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

The proving ground for the Astralis OS workspace, validating end-to-end security, capability isolation, and cross-component interactions.

`astrid-integration-tests` is a dedicated, private subcrate containing zero library code. Its sole purpose is to stitch together the disparate components of the Astralis OS — capabilities, capsules, MCP bridges, and the approval matrix — to ensure they function cohesively under hostile or complex conditions. It executes complete, real-world lifecycle scenarios against actual WebAssembly payloads and simulated user approval workflows to mathematically prove the runtime's security guarantees.

## Core Features

*   **WASM Capsule Sandboxing Validation**: End-to-end tests for memory allocation traps, IPC subscription limits, malicious payload rejection (like CRLF HTTP header injection), and Virtual File System (VFS) boundary enforcement.
*   **Security Interceptor Matrix**: Exhaustive verification of the `astrid-approval` interceptor, proving that budget constraints, capability tokens, and user policies interact correctly and atomically.
*   **MCP Host Engine Hardening**: Tests that validate strict capability checks when spawning native child processes or external MCP servers, ensuring no rogue binaries can be launched.
*   **Concurrency and Race Condition Proofs**: Atomic operations are hammered with highly concurrent tasks to guarantee no budget overspend and no double-consumption of single-use allowances.
*   **Allowance Scoping and Lifecycle**: Verification that session and workspace allowances remain securely isolated across boundaries, and that expired or out-of-scope allowances are aggressively cleaned up.

## Architecture

This crate relies on the `astrid-test` test harness and pre-compiled WebAssembly fixtures (such as `test-all-endpoints.wasm`) built by `astrid-openclaw`. Rather than mocking internal components, it instantiates the actual system engines:

1.  **Capsule Loading**: Real `CapsuleManifest` structures are fed into the `CapsuleLoader`, creating fully isolated Wasmtime environments.
2.  **Stateful Interception**: Tests construct a genuine `SecurityInterceptor` pipeline wired to in-memory `CapabilityStore`, `AllowanceStore`, and `AuditLog` instances.
3.  **Adversarial Emulation**: Test cases intentionally attempt path traversals (e.g., `../../../../etc/passwd`), memory limit violations, and unauthorized network egress to ensure the host engine traps or denies the request appropriately.
4.  **Deterministic Handlers**: The simulated human-in-the-loop is driven by deterministic `ApprovalHandler` implementations that conditionally return "Approve Session", "Approve Workspace", or "Deny", allowing complex multi-step approval workflows to be tested reliably.

## Quick Start

Because this crate runs the complete, un-mocked execution engine, it heavily relies on compiled fixtures. 

First, ensure the WASM test plugins are built (this typically happens automatically in the CI pipeline or via workspace scripts):
```bash
./scripts/compile-test-plugin.sh
```

Execute the full integration suite:
```bash
cargo test -p astrid-integration-tests
```

Run a specific test domain:
```bash
# Run only WASM capability isolation tests
cargo test -p astrid-integration-tests wasm_e2e

# Run only allowance and budget concurrency tests
cargo test -p astrid-integration-tests security_regressions
```

## Development

*   `src/lib.rs`: Minimal entrypoint enforcing workspace linting rules. Contains no functional code.
*   `tests/fixtures/`: Contains pre-compiled or statically defined manifests and WASM binaries used as test subjects.
*   `tests/wasm_e2e.rs`: Tests covering the `astrid-capsule` execution engine, focusing on capability enforcement, file system boundaries, and resource exhaustion traps.
*   `tests/mcp_e2e.rs`: Validates the MCP bridging layer and host process capability checking.
*   `tests/allowance_flow.rs`: Tests the state machine of user approvals, session caching, and workspace-scoped token generation.
*   `tests/security_regressions.rs`: Focused, high-concurrency tests proving atomic guarantees against previously known race conditions and architectural edge cases.

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
