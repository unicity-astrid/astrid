# astrid-config

[![Crates.io](https://img.shields.io/crates/v/astrid-config)](https://crates.io/crates/astrid-config)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Unified, layered, and strictly validated configuration for the Astrid secure agent runtime.

`astrid-config` consolidates all runtime configuration into a single `Config` type. It merges settings across up to five layers (embedded defaults, system, user, workspace, and environment variables) using a deterministic precedence chain. Critically, the workspace layer is treated as untrusted: it can only tighten security policies, never loosen them. Budget limits can only decrease, deny-lists can only grow, approval gates can only be enabled, and sensitive fields like API keys cannot be overridden. Post-merge validation catches invalid states before the runtime ever boots.

## Core Features

- **Deterministic five-layer precedence**: Embedded defaults, system (`/etc/astrid/config.toml`), user (`~/.astrid/config.toml`), workspace (`{workspace}/.astrid/config.toml`), and environment variable fallbacks merge in a predictable, documented order.
- **Workspace sandboxing with enforcement**: After merging the workspace layer, a hard enforcement pass reverts any attempt to expand budgets, loosen security booleans, add allow-list entries, inject trusted or auto-started servers, or override `model.api_key` / `model.api_url`. Violations are logged and silently reverted, not returned as errors.
- **Restricted `${VAR}` expansion**: String values in the workspace config can interpolate environment variables, but only `ASTRID_*` and `ANTHROPIC_*` prefixed names are permitted. References to arbitrary env vars like `${AWS_SECRET_ACCESS_KEY}` or `${HOME}` are left unresolved, preventing exfiltration via a malicious project config.
- **Strict post-merge validation**: After all layers are merged and env vars are applied, a validation pass checks supported providers, temperature range (0.0-1.0), token bounds (1-16,000,000), budget invariants (per-action must not exceed session), zero-value timeout guards, and valid enum strings for workspace mode, escape policy, git completion, log level, and log format.
- **Credentials never serialized**: `ModelConfig` has a custom `Serialize` implementation that omits `api_key` and `api_url` entirely. Its `Debug` implementation replaces values with presence booleans (`has_api_key: true`). `ServerSection` similarly redacts all environment variable values in `Debug` output.
- **Field-level provenance tracing**: `ResolvedConfig` carries a `FieldSources` map (`HashMap<String, ConfigLayer>`) recording which layer last wrote every dotted field path, enabling the `config show` command to annotate each value with `[defaults]`, `[system]`, `[user]`, `[workspace]`, or `[env]`.
- **No internal astrid dependencies**: The crate depends only on `serde`, `toml`, `serde_json`, `thiserror`, `tracing`, and `directories`. Domain type conversion happens at integration boundaries in other crates.

## Architecture: Precedence and Layering

From lowest to highest priority:

1. **Embedded defaults** (`defaults.toml` compiled into the binary via `include_str!`)
2. **System** (`/etc/astrid/config.toml`)
3. **User** (`~/.astrid/config.toml` or `$ASTRID_HOME/config.toml`)
4. **Workspace** (`{workspace}/.astrid/config.toml`) — with restriction enforcement
5. **Environment variables** (`ASTRID_*`, `ANTHROPIC_*`, `OPENAI_API_KEY`, `ZAI_API_KEY`) — fallback only, applied to fields not set by any file layer

The merge operates on raw `toml::Value` trees before deserialization, which correctly distinguishes "key absent" from "key present with default value". Only after all layers are merged and `${VAR}` references are resolved does the tree get deserialized into `Config`.

### Workspace Restriction Rules

The workspace layer is merged first, then an enforcement pass runs against the pre-workspace baseline:

| Field category | Rule |
|---|---|
| `budget.session_max_usd`, `budget.per_action_max_usd` | Can only decrease (clamped to baseline) |
| `budget.warn_at_percent` | Can only decrease |
| `security.policy.max_argument_size` | Can only decrease |
| `security.require_signatures` | Can only become `true` |
| `security.policy.require_approval_for_delete/network` | Can only become `true` |
| `security.policy.blocked_tools`, `denied_paths`, `denied_hosts`, `approval_required_tools` | Union only (workspace can add, not remove) |
| `security.policy.allowed_paths`, `allowed_hosts` | Cannot expand beyond baseline |
| `workspace.mode` | Can only tighten: `autonomous` < `guided` < `safe` |
| `workspace.escape_policy` | Can only tighten: `allow` < `ask` < `deny` |
| `workspace.never_allow` | Union only |
| `workspace.auto_allow_read`, `auto_allow_write` | Cannot expand beyond baseline |
| `model.api_key`, `model.api_url` | Cannot be set by workspace |
| `hooks.allow_wasm_hooks`, `allow_agent_hooks`, `allow_http_hooks`, `allow_command_hooks` | Can only become `false` |
| `rate_limits.*` | Can only decrease |
| `subagents.max_concurrent`, `max_depth`, `timeout_secs` | Can only decrease |
| `retry.llm_max_attempts`, `mcp_max_attempts` | Can only decrease |
| `timeouts.approval_secs`, `idle_secs` | Can only decrease |
| Workspace-injected MCP servers | Forced `trusted = false`, `auto_start = false` |
| Security fields on baseline servers (`command`, `args`, `env`, `cwd`, `binary_hash`, `trusted`) | Reverted to baseline values if workspace attempts to change them |

## Quick Start

Add `astrid-config` to your crate:

```toml
[dependencies]
astrid-config = { workspace = true }
```

### Load with Full Precedence Chain

```rust
use astrid_config::Config;
use std::path::Path;

let resolved = Config::load(Some(Path::new("/path/to/project")))?;

// Fully validated, merged configuration.
let config = &resolved.config;
println!("Provider: {}", config.model.provider);
println!("Session budget: ${}", config.budget.session_max_usd);

// Inspect which layer set a specific field.
if let Some(source) = resolved.field_sources.get("model.provider") {
    println!("model.provider came from: {source}");
}

// List every config file that was actually loaded.
for path in &resolved.loaded_files {
    println!("Loaded: {path}");
}
```

### Load with Explicit Home Override

Useful in tests or containers where the standard home directory detection should be bypassed:

```rust
use astrid_config::Config;
use std::path::Path;

let resolved = Config::load_with_home(
    Some(Path::new("/workspace")),
    Path::new("/custom/astrid/home"),
)?;
```

### Load a Single File (No Layering)

```rust
use astrid_config::Config;
use std::path::Path;

let config = Config::load_file(Path::new("custom.toml"))?;
```

### Show Resolved Configuration

```rust
use astrid_config::{Config, show::ShowFormat};
use std::path::Path;

let resolved = Config::load(None)?;

// TOML output with source annotations on each line.
let annotated = resolved.show(ShowFormat::Toml, None)?;
println!("{annotated}");

// JSON output of just the security section.
let json = resolved.show(ShowFormat::Json, Some("security"))?;
println!("{json}");
```

## API Reference

### Key Types

**`Config`** - Root configuration struct. All sections use `#[serde(default)]` so a bare `[section]` header in TOML produces a valid, fully-defaulted section.

| Field | Type | Description |
|---|---|---|
| `model` | `ModelConfig` | LLM provider, model name, API key (write-only), pricing |
| `runtime` | `RuntimeSection` | Context window, system prompt, summarization |
| `security` | `SecurityConfig` | Signatures, approval timeout, `PolicySection` |
| `budget` | `BudgetSection` | Session/per-action USD limits, warning threshold |
| `rate_limits` | `RateLimitsConfig` | Elicitation and pending request caps |
| `servers` | `HashMap<String, ServerSection>` | Named MCP server definitions |
| `audit` | `AuditConfig` | Audit log path and rotation size |
| `keys` | `KeysConfig` | ed25519 key file paths |
| `workspace` | `WorkspaceSection` | Boundary mode, escape policy, never-allow list |
| `git` | `GitConfig` | Completion strategy, auto-test, squash |
| `hooks` | `HooksSection` | Hook system master switch and per-type permissions |
| `logging` | `LoggingSection` | Level, format, per-crate directives |
| `gateway` | `GatewaySection` | Daemon state dir, hot-reload, health interval |
| `timeouts` | `TimeoutsSection` | Request, tool, subagent, MCP connect, approval, idle |
| `sessions` | `SessionsSection` | Per-user session cap, history limit, persistence |
| `subagents` | `SubagentsSection` | Concurrency cap, depth limit, timeout |
| `retry` | `RetrySection` | LLM and MCP retry attempts, backoff bounds |
| `spark` | `SparkSection` | Static agent identity seed (callsign, class, aura, signal, core) |
| `uplinks` | `Vec<UplinkConfig>` | Pre-declared uplink plugin declarations |
| `identity` | `IdentitySection` | Platform identity links applied at startup |

**`ResolvedConfig`** - Wraps `Config` with `field_sources: FieldSources` and `loaded_files: Vec<String>`. Produced by `Config::load` and `Config::load_with_home`.

**`ConfigLayer`** - Enum with variants `Defaults`, `System`, `User`, `Workspace`, `Environment`. Implements `Display` with human-readable descriptions.

**`ConfigError`** - Error variants: `ReadError` (I/O failure), `ParseError` (TOML parse failure), `ValidationError` (invariant violation), `EnvError` (env var problem), `RestrictionViolation`, `NoHomeDir`.

**`ShowFormat`** - `Toml` (with inline source annotations) or `Json` (for programmatic use).

### Environment Variables

Environment variables are fallbacks only. They are applied to fields that no file layer set. They are never applied on top of explicit config file values.

| Variable | Field |
|---|---|
| `ASTRID_MODEL_PROVIDER` | `model.provider` |
| `ASTRID_MODEL` | `model.model` |
| `ASTRID_MODEL_API_KEY` | `model.api_key` |
| `ASTRID_MODEL_API_URL` | `model.api_url` |
| `ASTRID_LOG_LEVEL` | `logging.level` |
| `ASTRID_BUDGET_SESSION_MAX_USD` | `budget.session_max_usd` |
| `ASTRID_BUDGET_PER_ACTION_MAX_USD` | `budget.per_action_max_usd` |
| `ASTRID_WORKSPACE_MODE` | `workspace.mode` |
| `ASTRID_SUBAGENT_MAX_CONCURRENT` | `subagents.max_concurrent` |
| `ASTRID_SUBAGENT_MAX_DEPTH` | `subagents.max_depth` |
| `ASTRID_SUBAGENT_TIMEOUT_SECS` | `subagents.timeout_secs` |
| `ASTRID_RETRY_LLM_MAX_ATTEMPTS` | `retry.llm_max_attempts` |
| `ASTRID_RETRY_MCP_MAX_ATTEMPTS` | `retry.mcp_max_attempts` |
| `ANTHROPIC_API_KEY` | `model.api_key` |
| `ANTHROPIC_MODEL` | `model.model` |
| `OPENAI_API_KEY` | `model.api_key` |
| `ZAI_API_KEY` | `model.api_key` |

`ASTRID_HOME` redirects user config discovery to an alternate directory. On Unix, the directory must be owned by the same UID as the home directory or it is rejected.

### MCP Server Configuration

Servers are defined as named tables. The `restart_policy` field accepts three forms:

```toml
[servers.my-server]
transport = "stdio"       # "stdio" | "sse" | "streamable-http"
command = "my-server-bin"
args = ["--flag"]
trusted = false           # false = WASM sandbox required
auto_start = true
binary_hash = "blake3:abc123..."   # optional integrity check
restart_policy = "never"  # simple form

# Retry form:
[servers.my-server.restart_policy]
on_failure = { max_retries = 5 }
```

Validation enforces that `stdio` transport requires `command` and that `sse` / `streamable-http` transports require `url`.

## Contributing

When adding configuration fields:

1. Add the field to the appropriate section struct in `src/types.rs` with a doc comment.
2. Provide a sensible, secure default in `src/defaults.toml`.
3. Add validation logic in `src/validate.rs`.
4. If the field is a security, budget, or limit field, add the corresponding clamp, union, or block rule in `src/merge/restrict.rs`.

## Development

```bash
cargo test -p astrid-config
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
