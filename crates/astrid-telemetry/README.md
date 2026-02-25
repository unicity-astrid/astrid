# astrid-telemetry

[![Crates.io](https://img.shields.io/crates/v/astrid-telemetry)](https://crates.io/crates/astrid-telemetry)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

High-performance, structured logging and distributed request correlation for the Astralis OS runtime.

`astrid-telemetry` provides the foundational observability layer for Astralis components. It wraps the Rust `tracing` ecosystem into a strictly typed, configurable API that enforces consistent output formats and enables cross-component request correlation. Instead of manually configuring complex subscriber pipelines in every crate, this library provides a unified interface to handle file rotation, multi-format output, and precise per-crate log directives.

## Core Features

* **Unified Configuration Pipeline**: Switch between JSON, pretty, compact, or full log formats through a simple builder or serializable configuration.
* **Distributed Request Correlation**: The `RequestContext` system automatically tracks parent-child relationships, correlation IDs, and operational metadata across system boundaries.
* **Automated Lifecycle Tracing**: `RequestGuard` inherently records entry and exit spans, calculating precise execution duration metrics upon drop.
* **Flexible Output Routing**: Route traces to standard output, standard error, or rotating file appenders (daily, hourly, minutely, or never).
* **Dynamic Directive Filtering**: Filter telemetry data at runtime using standard directive syntax (e.g., `astrid_mcp=debug,astrid_core=trace`).

## Architecture

`astrid-telemetry` abstracts the underlying complexities of `tracing-subscriber` to ensure reliable observability. 

1. **Routing**: `LogTarget` dictates the physical destination (`Stdout`, `Stderr`, `File`).
2. **Formatting**: Layers are dynamically applied via `tracing_subscriber::fmt` based on the chosen `LogFormat`.
3. **Filtering**: `EnvFilter` is constructed from the base log level and the vector of directive overrides, resolving parse errors gracefully.

By centralizing these concerns, all components within the Astralis ecosystem guarantee a uniform logging topology, preventing fragmented observability configurations across the codebase.

## Quick Start

Initialize the basic telemetry system at the entry point of your application or agent.

```rust
use astrid_telemetry::{LogConfig, LogFormat, setup_logging};

fn main() -> Result<(), astrid_telemetry::TelemetryError> {
    // Configure human-readable output with specific module tracing
    let config = LogConfig::new("info")
        .with_format(LogFormat::Pretty)
        .with_directive("astrid_mcp=trace");

    // Initialize the global subscriber
    setup_logging(&config)?;

    tracing::info!("Telemetry subsystem initialized");
    Ok(())
}
```

### Logging Configuration

The `LogConfig` struct uses a fluent builder pattern to define the output format and routing of telemetry data. It implements `Serialize` and `Deserialize`, making it trivial to load from a central configuration file.

```rust
use astrid_telemetry::{LogConfig, LogTarget, LogFormat, FileRotation};

let config = LogConfig::new("warn")
    // Use structured JSON for machine ingestion
    .with_format(LogFormat::Json)
    // Route logs to a rotating file appender
    .with_file_logging_rotation("/var/log/astralis", "agent", FileRotation::Daily)
    // Inject file names and line numbers into the trace
    .with_file_info()
    // Explicitly enable debug logs for the storage layer
    .with_directive("astrid_storage=debug");
```

Supported formats:
* `Pretty`: Human-readable format with ANSI colors (default).
* `Compact`: Dense, single-line format optimized for terminal real estate.
* `Json`: Structured JSON output designed for log aggregation systems.
* `Full`: Exhaustive format including all available span and event fields.

### Request Correlation

In a distributed runtime, understanding how requests flow between components is critical. `RequestContext` generates and propagates correlation IDs.

```rust
use astrid_telemetry::RequestContext;
use uuid::Uuid;

// 1. A new request enters the system
let session_id = Uuid::new_v4();
let root_ctx = RequestContext::new("api_gateway")
    .with_operation("handle_client_request")
    .with_session_id(session_id);

// 2. The request is passed to a subsystem
// The child inherits the correlation ID and session ID, but receives a unique request ID
let storage_ctx = root_ctx.child("storage_engine")
    .with_operation("fetch_user_data");

// 3. Bind the context to the tracing span
let span = storage_ctx.span();
let _guard = span.enter();

tracing::info!("Executing storage lookup"); // Automatically tagged with correlation IDs
```

### Execution Guards

To automatically track the lifecycle and duration of an operation, use `RequestGuard`. It emits debug logs when a request starts and, upon being dropped, logs the completion along with the total elapsed time in milliseconds.

```rust
use astrid_telemetry::{RequestContext, RequestGuard};

fn process_data() {
    let ctx = RequestContext::new("data_processor")
        .with_operation("crunch_numbers");
    
    // The guard emits a "Request started" debug log and creates an active span
    let _guard = RequestGuard::new(ctx);
    
    // ... perform complex operations ...
    
    // When _guard drops out of scope, it emits a "Request completed" debug log
    // containing the `elapsed_ms` field
}
```

## Development

This crate is a core component within the Astralis workspace. To run the isolated test suite:

```bash
cargo test -p astrid-telemetry
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
