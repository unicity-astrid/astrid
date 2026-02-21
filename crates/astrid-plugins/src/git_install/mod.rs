//! Git-based plugin installation.
//!
//! Supports two source formats:
//! - `github:org/repo[@ref]` — fetches via GitHub tarball API
//! - `git:https://host/path.git[@ref]` — clones via `git clone --depth=1`
//!
//! After fetching, the source is extracted into a temporary directory and
//! returned for the caller to detect the plugin type and route to the
//! appropriate install pipeline.



pub mod source;
pub mod validate;
pub mod fetch;
#[cfg(test)]
pub mod tests;

pub use source::GitSource;
#[cfg(feature = "http")]
pub use fetch::fetch_git_source;
