//! Astralis Telemetry - Logging and tracing for the Astralis secure agent runtime SDK.
//!
//! This crate provides:
//! - Configurable logging setup with multiple formats
//! - Request context for correlation across operations
//! - Integration with the tracing ecosystem
//!
//! # Example
//!
//! ```rust,no_run
//! use astralis_telemetry::{LogConfig, LogFormat, setup_logging, RequestContext};
//!
//! # fn main() -> Result<(), astralis_telemetry::TelemetryError> {
//! // Set up logging
//! let config = LogConfig::new("debug")
//!     .with_format(LogFormat::Pretty)
//!     .with_directive("astralis_mcp=trace");
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
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

pub mod prelude;

mod context;
mod error;
mod logging;

pub use context::{RequestContext, RequestGuard};
pub use error::{TelemetryError, TelemetryResult};
pub use logging::{LogConfig, LogFormat, LogTarget, setup_default_logging, setup_logging};
