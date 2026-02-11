# astralis-telemetry

Logging and tracing for the Astralis secure agent runtime SDK.

## Features

- **Configurable Logging**: Multiple output formats (Pretty, Compact, JSON, Full)
- **Flexible Targets**: Output to stdout, stderr, or files
- **Request Context**: Correlation IDs for tracing requests across operations
- **Tracing Integration**: Built on the `tracing` ecosystem
- **Builder Pattern**: Fluent API for configuration
- **Serializable Config**: Load logging configuration from JSON/TOML

## Usage

```rust
use astralis_telemetry::{LogConfig, LogFormat, setup_logging, RequestContext};

fn main() -> Result<(), astralis_telemetry::TelemetryError> {
    // Set up logging with custom configuration
    let config = LogConfig::new("debug")
        .with_format(LogFormat::Pretty)
        .with_directive("astralis_mcp=trace");

    setup_logging(&config)?;

    // Create a request context for correlation
    let ctx = RequestContext::new("my_component")
        .with_operation("process_request");

    // Use the context's span for tracing
    let span = ctx.span();
    let _guard = span.enter();
    tracing::info!("Processing request");

    Ok(())
}
```

## Log Formats

| Format | Description |
|--------|-------------|
| `Pretty` | Human-readable with colors (default) |
| `Compact` | Single-line format |
| `Json` | Structured JSON for log aggregation |
| `Full` | All fields included |

## Request Context

`RequestContext` provides correlation across distributed operations:

```rust
use astralis_telemetry::RequestContext;

let ctx = RequestContext::new("api")
    .with_operation("handle_request")
    .with_session_id(session_id)
    .with_user_id(user_id);

// Create child contexts that inherit correlation IDs
let child_ctx = ctx.child("database");
```

## License

This crate is licensed under the MIT license.
