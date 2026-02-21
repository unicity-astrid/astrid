//! Git-based plugin installation.
//!
//! Supports two source formats:
//! - `github:org/repo[@ref]` — fetches via GitHub tarball API
//! - `git:https://host/path.git[@ref]` — clones via `git clone --depth=1`
//!
//! After fetching, the source is extracted into a temporary directory and
//! returned for the caller to detect the plugin type and route to the
//! appropriate install pipeline.

/// Network retrieval and unpacking primitives.
pub mod fetch;
/// Git source definitions and primitives.
pub mod source;
#[cfg(test)]
mod tests;
/// Security validation utilities for strings.
pub mod validate;

#[cfg(feature = "http")]
pub use fetch::fetch_git_source;
pub use source::GitSource;
