# astralis-prelude

Unified prelude for the Astralis secure agent runtime SDK.

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
astralis-prelude.workspace = true
```

Then import everything commonly needed:

```rust
use astralis_prelude::*;
```

## What's Included

This crate re-exports the preludes from all Astralis SDK crates:

| Crate | Types |
|-------|-------|
| `astralis-core` | `Frontend`, `SecurityError`, `SessionId`, `Permission`, `RiskLevel` |
| `astralis-crypto` | `KeyPair`, `PublicKey`, `Signature`, `ContentHash` |
| `astralis-capabilities` | `CapabilityToken`, `CapabilityStore`, `ResourcePattern` |
| `astralis-audit` | `AuditLog`, `AuditAction`, `AuditEntry`, `AuthorizationProof` |
| `astralis-mcp` | `McpClient`, `ServerConfig`, `ToolDefinition` |
| `astralis-runtime` | `AgentRuntime`, `RuntimeConfig`, `AgentSession`, `SessionStore` |
| `astralis-llm` | `LlmProvider`, `ClaudeProvider`, `Message`, `StreamEvent` |
| `astralis-events` | `EventBus`, `EventReceiver`, `AstralisEvent` |
| `astralis-hooks` | `Hook`, `HookManager`, `HookEvent`, `HookHandler` |
| `astralis-workspace` | `WorkspaceBoundary`, `WorkspaceConfig`, `WorkspaceMode` |
| `astralis-telemetry` | `LogConfig`, `LogFormat`, `setup_logging`, `RequestContext` |

## Per-Crate Preludes

If you only need types from specific crates, use their individual preludes instead:

```rust
use astralis_core::prelude::*;
use astralis_crypto::prelude::*;
```

This avoids pulling in unnecessary dependencies and keeps compile times fast.
