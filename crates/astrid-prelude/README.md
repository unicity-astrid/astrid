# astrid-prelude

[![Crates.io](https://img.shields.io/crates/v/astrid-prelude)](https://crates.io/crates/astrid-prelude)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Within the Astralis OS architecture, `astrid-prelude` serves as the centralized namespace and orchestration layer for workspace-wide types. The system is intentionally fractured into highly isolated, domain-specific crates to enforce strict security and capability boundaries. This crate bridges the gap for high-level consumers—such as daemons, CLI frontends, and integration tests—by aggregating the public API surface of the entire OS into a single, cohesive import without compromising the underlying micro-kernel isolation.

## Core Features

*   **Unified Namespace**: Brings the core building blocks of Astralis (agents, cryptography, capability tokens, and audit logs) into scope instantly.
*   **Ergonomic Orchestration**: Simplifies the bootstrapping of complex runtime environments by exposing standard configuration types alongside operational primitives.
*   **Zero-Cost Abstraction**: Acts strictly as a re-export layer containing no runtime logic, ensuring it introduces zero overhead and does not bypass architectural boundaries.
*   **Granular Fallback**: Designed to be complementary; lower-level crates can bypass the global prelude and use domain-specific preludes (e.g., `astrid_core::prelude::*`) to maintain minimal dependency footprints.

## Architecture

In the Astralis OS, functionality is strictly compartmentalized to guarantee security and auditability. The capability system has no knowledge of the LLM provider, and the cryptographic layer operates entirely independent of the MCP client. 

While this compartmentalization is a strict requirement for the system's security model, it creates friction when building high-level orchestrators (like the CLI, the gateway daemon, or user-facing bots) that must coordinate these disparate pieces. `astrid-prelude` serves as the developer-facing aggregation point. It provides the ergonomics of a monolith for application-layer development while preserving the strict security boundaries and isolation of the underlying crates.

`astrid-prelude` systematically re-exports the `prelude::*` modules from the foundational crates of the Astralis OS:

*   **`astrid_audit`**: Cryptographically secure action logging and authorization proofs.
*   **`astrid_capabilities`**: Token-based permission management and resource access patterns.
*   **`astrid_core`**: Foundational types, unified error handling (`RuntimeResult`), and identity.
*   **`astrid_crypto`**: Key pairs, cryptographic signatures, and hashing primitives.
*   **`astrid_events`**: Event bus architecture and asynchronous message passing types.
*   **`astrid_kernel`**: Daemon layer communication, routing, and IPC abstractions.
*   **`astrid_hooks`**: Interception logic and lifecycle hook management.
*   **`astrid_llm`**: Model providers, stream events, and message abstractions.
*   **`astrid_mcp`**: Model Context Protocol clients, tools, and server configurations.
*   **`astrid_runtime`**: Core execution loops, sessions, and state management.
*   **`astrid_telemetry`**: Tracing, structured diagnostics, and log configuration.
*   **`astrid_workspace`**: Filesystem boundary enforcement and access control definitions.

## Quick Start

Add the dependency to your crate's `Cargo.toml` utilizing workspace inheritance:

```toml
[dependencies]
astrid-prelude = { workspace = true }
```

Bring the Astralis OS ecosystem into scope:

```rust
use astrid_prelude::*;

// Types from astrid-core, astrid-crypto, astrid-runtime, etc., are now available
```

The primary use case for `astrid-prelude` is bootstrapping the agent runtime and wiring together disparate components that span multiple security boundaries. 

The following example demonstrates how the prelude provides all necessary primitives to initialize an audited, capability-backed LLM execution environment:

```rust
use astrid_prelude::*;

async fn initialize_agent_runtime() -> RuntimeResult<()> {
    // 1. Cryptography and identity (astrid-crypto)
    let runtime_key = KeyPair::generate();
    let audit_key = KeyPair::generate();

    // 2. Secure audit logging (astrid-audit)
    let audit = AuditLog::in_memory(audit_key);

    // 3. Model Context Protocol integration (astrid-mcp)
    let mcp = McpClient::from_default_config()?;

    // 4. Intelligence provider configuration (astrid-llm)
    let llm = ClaudeProvider::new(
        ProviderConfig::new("api-key", "claude-sonnet-4-20250514")
    );

    // 5. Core OS configuration and instantiation (astrid-core & astrid-runtime)
    let home = astrid_core::dirs::AstridHome::resolve()?;
    let sessions = SessionStore::from_home(&home);
    
    let runtime = AgentRuntime::new(
        llm,
        mcp,
        audit,
        sessions,
        runtime_key,
        RuntimeConfig::default(),
    );

    Ok(())
}
```

## Development

This crate is structurally simple and contains no discrete logic. Its correctness is implicitly verified by the compiler ensuring that all re-exported symbols are valid and visible.

To verify the prelude and its generated documentation:

```bash
cargo check -p astrid-prelude
cargo doc -p astrid-prelude --no-deps --open
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
