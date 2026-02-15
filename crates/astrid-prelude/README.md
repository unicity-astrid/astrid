# astrid-prelude

Unified prelude for the Astrid secure agent runtime SDK.

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
astrid-prelude.workspace = true
```

Then import everything commonly needed:

```rust
use astrid_prelude::*;
```

## What's Included

This crate re-exports the preludes from all Astrid SDK crates:

| Crate | Types |
|-------|-------|
| `astrid-core` | `Frontend`, `SecurityError`, `SessionId`, `Permission`, `RiskLevel` |
| `astrid-crypto` | `KeyPair`, `PublicKey`, `Signature`, `ContentHash` |
| `astrid-capabilities` | `CapabilityToken`, `CapabilityStore`, `ResourcePattern` |
| `astrid-audit` | `AuditLog`, `AuditAction`, `AuditEntry`, `AuthorizationProof` |
| `astrid-mcp` | `McpClient`, `ServerConfig`, `ToolDefinition` |
| `astrid-runtime` | `AgentRuntime`, `RuntimeConfig`, `AgentSession`, `SessionStore` |
| `astrid-llm` | `LlmProvider`, `ClaudeProvider`, `Message`, `StreamEvent` |
| `astrid-events` | `EventBus`, `EventReceiver`, `AstridEvent` |
| `astrid-hooks` | `Hook`, `HookManager`, `HookEvent`, `HookHandler` |
| `astrid-workspace` | `WorkspaceBoundary`, `WorkspaceConfig`, `WorkspaceMode` |
| `astrid-telemetry` | `LogConfig`, `LogFormat`, `setup_logging`, `RequestContext` |

## Per-Crate Preludes

If you only need types from specific crates, use their individual preludes instead:

```rust
use astrid_core::prelude::*;
use astrid_crypto::prelude::*;
```

This avoids pulling in unnecessary dependencies and keeps compile times fast.
