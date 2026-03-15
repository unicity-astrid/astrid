# astrid-telemetry

[![Crates.io](https://img.shields.io/crates/v/astrid-telemetry)](https://crates.io/crates/astrid-telemetry)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Structured logging and request correlation for the Astrid secure agent runtime.

`astrid-telemetry` wraps the `tracing` ecosystem into a typed, serializable configuration API. Rather than wiring up `tracing-subscriber` in every crate, components call `setup_logging` once and get consistent output format, file rotation, and per-target directive filtering. The `RequestContext` type propagates correlation IDs across component boundaries so distributed call chains remain traceable in any output format.

## Core Features

- **Four output formats**: `Pretty` (ANSI-colored, human-readable), `Compact` (single-line), `Json` (machine-ingestible), `Full` (all span fields). Switch with a single enum variant.
- **Three output targets**: stdout, stderr, or a rolling file appender. File output automatically disables ANSI codes.
- **File rotation strategies**: daily, hourly, minutely, or never - backed by `tracing-appender`.
- **Per-target directive overrides**: apply `astrid_mcp=debug,astrid_core=trace`-style filters on top of the base level via `with_directive`.
- **Request correlation**: `RequestContext` carries `request_id`, `correlation_id`, `parent_id`, `session_id`, `user_id`, elapsed time, and arbitrary string metadata. Child contexts inherit correlation and session IDs while getting a fresh `request_id`.
- **Serializable config**: `LogConfig` implements `serde::Serialize` / `Deserialize`, so it loads directly from any config file format the rest of the workspace uses.
- **Prelude**: `use astrid_telemetry::prelude::*` imports all essential types in one line.

## Quick Start

```toml
[dependencies]
astrid-telemetry = "0.2"
```

Initialize once at your entry point:

```rust
use astrid_telemetry::{LogConfig, LogFormat, setup_logging};

fn main() -> Result<(), astrid_telemetry::TelemetryError> {
    let config = LogConfig::new("info")
        .with_format(LogFormat::Pretty)
        .with_directive("astrid_mcp=trace");

    setup_logging(&config)?;

    tracing::info!("runtime started");
    Ok(())
}
```

## API Reference

### Key Types

#### `LogConfig`

Builder-style configuration for the global tracing subscriber. All fields implement `Serialize` / `Deserialize`.

```rust
use astrid_telemetry::{LogConfig, LogFormat, LogTarget, FileRotation};

// JSON to a daily-rotating file, with source locations included
let config = LogConfig::new("warn")
    .with_format(LogFormat::Json)
    .with_file_logging_rotation("/var/log/astrid", "agent", FileRotation::Daily)
    .with_file_info()
    .with_directive("astrid_storage=debug");
```

| Builder method | Effect |
|---|---|
| `with_format(LogFormat)` | Set output format |
| `with_target(LogTarget)` | Set output target |
| `with_file_logging(dir, prefix)` | Route to daily-rotating file |
| `with_file_logging_rotation(dir, prefix, rotation)` | Route to file with explicit rotation |
| `with_directive(str)` | Append a filter directive |
| `with_file_info()` | Include file name and line number |
| `with_span_events()` | Emit `NEW` and `CLOSE` span events |
| `without_timestamps()` | Suppress timestamps |
| `without_ansi()` | Disable ANSI color codes |

#### `LogFormat`

```rust
pub enum LogFormat { Pretty, Compact, Json, Full }
```

Default is `Pretty`. `Json` is the right choice for log aggregation pipelines.

#### `LogTarget`

```rust
pub enum LogTarget { Stdout, Stderr, File(PathBuf) }
```

Default is `Stderr`.

#### `FileRotation`

```rust
pub enum FileRotation { Daily, Hourly, Minutely, Never }
```

Default is `Daily`. `Minutely` is useful in tests to observe rotation behavior without waiting.

#### `FileLogConfig`

Embedded in `LogConfig` when the target is `File`. Fields:

| Field | Type | Default |
|---|---|---|
| `directory` | `PathBuf` | `"logs"` |
| `prefix` | `String` | `"astrid"` |
| `rotation` | `FileRotation` | `Daily` |
| `max_files` | `usize` | `0` (unlimited) |

#### `RequestContext`

Carries correlation state across component and async boundaries.

```rust
use astrid_telemetry::RequestContext;
use uuid::Uuid;

// Root context for an incoming request
let root = RequestContext::new("api_gateway")
    .with_operation("handle_request")
    .with_session_id(Uuid::new_v4());

// Child inherits correlation_id and session_id; gets a new request_id and parent_id
let child = root.child("storage")
    .with_operation("fetch_record");

// Attach to a tracing span
let span = child.span();
let _guard = span.enter();
tracing::info!("querying storage");

// Elapsed time since context creation
println!("{}ms", child.elapsed_ms());
```

Fields on `RequestContext`:

| Field | Type | Notes |
|---|---|---|
| `request_id` | `Uuid` | Unique per context |
| `correlation_id` | `Uuid` | Shared across a request chain |
| `parent_id` | `Option<Uuid>` | Set on child contexts |
| `session_id` | `Option<Uuid>` | Inherited by children |
| `user_id` | `Option<Uuid>` | Optional, not inherited |
| `source` | `String` | Component that created the context |
| `operation` | `Option<String>` | Name of the operation being performed |
| `metadata` | `HashMap<String, String>` | Inherited by children |
| `started_at` | `DateTime<Utc>` | Set at creation time |

`RequestContext` implements `Serialize` / `Deserialize` and `Default` (source `"unknown"`).

#### `TelemetryError`

```rust
pub enum TelemetryError {
    ConfigError(String),   // invalid level string or directive syntax
    InitError(String),     // subscriber already set or init failed
    IoError(std::io::Error),
}
```

`TelemetryResult<T>` is `Result<T, TelemetryError>`.

### `setup_logging`

```rust
pub fn setup_logging(config: &LogConfig) -> TelemetryResult<()>
```

Installs the global `tracing` subscriber. Call once per process. Returns `TelemetryError::InitError` if a subscriber is already installed (which is normal in tests - use `try_init` semantics by ignoring the error where appropriate).

## Development

```bash
cargo test -p astrid-telemetry
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
