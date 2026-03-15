# astrid-telemetry

[![Crates.io](https://img.shields.io/crates/v/astrid-telemetry)](https://crates.io/crates/astrid-telemetry)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The structured logging layer of the Astrid OS.**

Every kernel and every daemon needs observability. This crate wraps the `tracing` ecosystem into a typed, serializable configuration API. Components call `setup_logging` once and get consistent output format, file rotation, and per-target directive filtering. `RequestContext` propagates correlation IDs across async boundaries so distributed call chains remain traceable in any output format.

## Why it exists

Astrid has many crates that all emit tracing events: the kernel, the capsule runtime, the MCP client, the approval system. Without a shared logging configuration, each crate invents its own setup. This crate centralizes that into a single `LogConfig` struct that implements `Serialize`/`Deserialize`, loads from any config file format, and produces a consistent subscriber across the entire process.

The `RequestContext` exists because agent operations fan out. A single user message triggers orchestration, tool calls, approval checks, and audit writes. Correlation IDs tie these together so `grep` across JSON logs works.

## What it provides

- **Four output formats.** `Pretty` (ANSI-colored), `Compact` (single-line), `Json` (machine-ingestible), `Full` (all span fields). Switch with a single enum variant.
- **Three output targets.** Stdout, stderr, or a rolling file appender. File output automatically disables ANSI codes.
- **File rotation.** Daily, hourly, minutely, or never. Backed by `tracing-appender`.
- **Per-target directives.** Apply `astrid_mcp=debug,astrid_core=trace`-style filters on top of the base level.
- **Request correlation.** `RequestContext` carries `request_id`, `correlation_id`, `parent_id`, `session_id`, elapsed time, and arbitrary metadata. `.child(source)` creates a correlated child context with a fresh `request_id`. `.span()` attaches the context to a tracing span.
- **Serializable config.** `LogConfig` round-trips through JSON, TOML, or any serde format.

## Quick start

```toml
[dependencies]
astrid-telemetry = "0.2"
```

```rust
use astrid_telemetry::{LogConfig, LogFormat, setup_logging};

let config = LogConfig::new("info")
    .with_format(LogFormat::Pretty)
    .with_directive("astrid_mcp=trace");

setup_logging(&config)?;
```

## Development

```bash
cargo test -p astrid-telemetry
```

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
