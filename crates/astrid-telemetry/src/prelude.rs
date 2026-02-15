//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astrid_telemetry::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust,no_run
//! use astrid_telemetry::prelude::*;
//!
//! # fn main() -> TelemetryResult<()> {
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

// Errors
pub use crate::{TelemetryError, TelemetryResult};

// Logging configuration
pub use crate::{LogConfig, LogFormat, LogTarget};

// Setup functions
pub use crate::{setup_default_logging, setup_logging};

// Request context
pub use crate::{RequestContext, RequestGuard};
