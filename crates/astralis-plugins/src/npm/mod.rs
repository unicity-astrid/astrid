//! npm registry HTTP fetcher for `OpenClaw` plugin absorption.
//!
//! Downloads packages directly from the npm registry via HTTP — without using
//! the npm CLI — to eliminate lifecycle script attack vectors (CVE with CVSS 9.6).
//!
//! # Pipeline
//!
//! 1. Parse npm spec (`@scope/name@version`)
//! 2. Fetch package metadata from registry
//! 3. Download tarball with size limits
//! 4. Verify SHA-512 SRI integrity
//! 5. Extract safely with path traversal protection
//! 6. Validate `openclaw.plugin.json` exists
//!
//! # Security
//!
//! - **No npm CLI**: Pure HTTP fetch eliminates lifecycle script attacks
//! - **SRI verification**: SHA-512 integrity check before extraction
//! - **Path traversal protection**: All archive entries validated
//! - **Size limits**: Configurable max tarball size (default 50 MB)
//! - **Entry count limits**: Max 10,000 files per archive
//! - **WASM sandbox**: All extracted code runs in WASM sandbox

pub mod extract;
pub mod fetcher;
pub mod integrity;
pub mod spec;
pub mod types;

pub use fetcher::{ExtractedPackage, NpmFetcher};
pub use spec::NpmSpec;
