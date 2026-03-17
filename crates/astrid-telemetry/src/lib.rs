//! Astrid Telemetry - Logging and tracing for the Astrid secure agent runtime.
//!
//! This crate provides:
//! - Configurable logging setup with multiple formats
//! - Request context for correlation across operations
//! - Integration with the tracing ecosystem
//!
//! # Example
//!
//! ```rust,no_run
//! use astrid_telemetry::{LogConfig, LogFormat, setup_logging, RequestContext};
//!
//! # fn main() -> Result<(), astrid_telemetry::TelemetryError> {
//! // Set up logging
//! let config = LogConfig::new("debug")
//!     .with_format(LogFormat::Pretty)
//!     .with_directive("astrid_mcp=trace");
//!
//! setup_logging(&config)?;
//!
//! // Create a request context
//! let ctx = RequestContext::new("my_component")
//!     .with_operation("process_request");
//!
//! // Use the context's span for tracing
//! let span = ctx.span();
//! let _guard = span.enter();
//! tracing::info!("Processing request");
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod prelude;

mod context;
mod error;
mod logging;

pub use context::RequestContext;
pub use error::{TelemetryError, TelemetryResult};
pub use logging::{LogConfig, LogFormat, LogTarget, setup_logging};

/// Convert an [`astrid_config::Config`] into a [`LogConfig`] for telemetry init.
///
/// Available when the `config` feature is enabled.
#[cfg(feature = "config")]
#[must_use]
pub fn log_config_from(cfg: &astrid_config::Config) -> LogConfig {
    let format = match cfg.logging.format.as_str() {
        "pretty" => LogFormat::Pretty,
        "json" => LogFormat::Json,
        "full" => LogFormat::Full,
        _ => LogFormat::Compact,
    };
    LogConfig {
        level: cfg.logging.level.clone(),
        format,
        directives: cfg.logging.directives.clone(),
        ..Default::default()
    }
}
