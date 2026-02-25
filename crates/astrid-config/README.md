# astrid-config

[![Crates.io](https://img.shields.io/crates/v/astrid-config)](https://crates.io/crates/astrid-config)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Unified, layered, and strictly validated configuration for the Astralis OS runtime.

`astrid-config` provides a single, consolidated configuration type for the entire Astrid runtime. It resolves configuration scattered across domains (security, budgets, models, gateways) into a cohesive, deterministic structure. Rather than relying on simple overrides, it implements a secure merging algorithm where lower-trust scopes (like a project workspace) are cryptographically restricted from loosening security policies established by higher-trust scopes (like the system administrator).

## Core Features

* **Deterministic Layering**: Predictable merging of embedded defaults, system, user, and workspace configurations.
* **Security Enforcement**: Workspace-level configurations are sandboxed. They can tighten budgets, reduce rate limits, and append to blocklists, but they cannot exfiltrate API keys, elevate workspace modes, or expand allowed system paths.
* **Strict Validation**: Post-merge validation guarantees invariants (e.g., maximum token boundaries, supported providers, non-negative limits) before the runtime boots, making invalid states unrepresentable.
* **Traceability**: The `ResolvedConfig` struct explicitly tracks the origin file of every final field value (`FieldSources`), simplifying debugging.
* **Zero Internal Dependencies**: Designed as a pure data crate. It depends only on `serde`, `toml`, `directories`, and `thiserror`, with domain conversion happening strictly at the integration boundary to minimize coupling.

## Architecture: Precedence & Layering

From lowest to highest priority, `astrid-config` merges settings in the following order:

1. **Embedded Defaults** (`defaults.toml` compiled directly into the binary)
2. **System** (`/etc/astrid/config.toml`)
3. **User** (`~/.astrid/config.toml` or `$ASTRID_HOME/config.toml`)
4. **Workspace** (`{workspace}/.astrid/config.toml`)
5. **Environment Variables** (e.g., `ASTRID_MODEL_PROVIDER`, used as final fallbacks)

### Workspace Security Restrictions

The workspace layer is treated as untrusted. When merging the workspace configuration, `astrid-config` enforces strict boundary rules against the system/user baseline:

* **Clamping**: Budgets, timeouts, concurrency limits, and rate limits can only be decreased.
* **Monotonic Security**: Security toggles (e.g., `require_approval_for_network`) can only be enabled, never disabled.
* **Union-Only Arrays**: Deny-lists (`blocked_tools`, `denied_paths`) can only accept new entries.
* **Expansion Blocking**: Allow-lists (`allowed_paths`, `allowed_hosts`) cannot be expanded beyond the baseline.
* **Override Blocking**: Sensitive fields like `model.api_key` or `model.api_url` cannot be redefined by the workspace.

## Quick Start

Add `astrid-config` to your crate dependencies:

```toml
[dependencies]
astrid-config = { workspace = true }
```

### Loading the Full Precedence Chain

```rust
use astrid_config::Config;
use std::path::Path;

// Load the configuration, automatically detecting defaults, system, user,
// and merging the provided workspace root.
let workspace_root = Path::new("/path/to/project");
let resolved = Config::load(Some(workspace_root)).expect("Failed to load configuration");

// Access the strictly validated configuration
let config = resolved.config;
println!("Active Provider: {}", config.model.provider);

// Inspect where a specific field came from
let provider_source = resolved.field_sources.get("model.provider");
println!("Provider configured by layer: {:?}", provider_source);
```

### Loading a Specific File

For isolated contexts, tooling, or testing, you can bypass the layering logic and load a single file directly:

```rust
use astrid_config::Config;
use std::path::Path;

let config = Config::load_file(Path::new("custom-config.toml")).expect("Failed to parse config file");
```

## Development

The public API is deliberately small, exporting the core types required for integration:

* `Config`: The root data structure containing all sections (`ModelConfig`, `SecurityConfig`, `BudgetSection`, etc.).
* `ResolvedConfig`: A wrapper around `Config` that includes file loading history and field-level lineage.
* `ConfigLayer`: An enum representing the origin of a configuration value (Defaults, System, User, Workspace, Environment).
* `ConfigError`: A unified error type encompassing IO errors, TOML parsing failures, and constraint violations.

To run tests specific to the configuration parser, validation bounds, and merge restriction logic:

```bash
cargo test -p astrid-config
```

When contributing to this crate, adhere to the following workflow:
1. Add new configuration fields to the appropriate section in `src/types.rs`.
2. Provide a sensible, secure fallback for the field in `src/defaults.toml`.
3. Add strict validation logic in `src/validate.rs`. 
4. If the field pertains to security, budgets, or limits, appropriate clamp, union, or block logic MUST be added to `src/merge/restrict.rs` to ensure workspace sandboxing holds.

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
