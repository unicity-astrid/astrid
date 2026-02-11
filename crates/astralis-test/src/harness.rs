//! Test harness helpers.

use std::path::PathBuf;
use tempfile::{NamedTempFile, TempDir};
use tracing_subscriber::EnvFilter;

/// Create a temporary directory for testing.
///
/// The directory is automatically cleaned up when the returned `TempDir` is dropped.
///
/// # Panics
///
/// Panics if the temporary directory cannot be created.
#[must_use]
pub fn test_dir() -> TempDir {
    TempDir::new().expect("Failed to create temp directory")
}

/// Create a temporary directory with a specific prefix.
///
/// # Panics
///
/// Panics if the temporary directory cannot be created.
#[must_use]
pub fn test_dir_with_prefix(prefix: &str) -> TempDir {
    TempDir::with_prefix(prefix).expect("Failed to create temp directory")
}

/// Create a temporary file with the given content.
///
/// Returns the `NamedTempFile` which will be cleaned up when dropped.
///
/// # Panics
///
/// Panics if the file cannot be created or written.
#[must_use]
pub fn test_file(content: &str) -> NamedTempFile {
    use std::io::Write;

    let mut file = NamedTempFile::new().expect("Failed to create temp file");
    file.write_all(content.as_bytes())
        .expect("Failed to write to temp file");
    file.flush().expect("Failed to flush temp file");
    file
}

/// Create a temporary file with a specific extension.
///
/// # Panics
///
/// Panics if the file cannot be created or written.
#[must_use]
pub fn test_file_with_extension(content: &str, extension: &str) -> NamedTempFile {
    use std::io::Write;

    let mut file = tempfile::Builder::new()
        .suffix(&format!(".{extension}"))
        .tempfile()
        .expect("Failed to create temp file");
    file.write_all(content.as_bytes())
        .expect("Failed to write to temp file");
    file.flush().expect("Failed to flush temp file");
    file
}

/// Create a file within a temporary directory.
///
/// Returns the path to the created file.
///
/// # Panics
///
/// Panics if the file cannot be created or written.
#[must_use]
pub fn test_file_in_dir(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    use std::fs;

    let path = dir.path().join(name);

    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("Failed to create parent directories");
    }

    fs::write(&path, content).expect("Failed to write file");
    path
}

/// Set up test logging with the given filter.
///
/// This initializes the tracing subscriber for tests. Should be called
/// at the beginning of tests that need logging.
///
/// # Example
///
/// ```rust,ignore
/// use astralis_test::setup_test_logging;
///
/// #[test]
/// fn my_test() {
///     setup_test_logging("debug");
///     // ... test code
/// }
/// ```
pub fn setup_test_logging(filter: &str) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_test_writer()
        .try_init();
}

/// Set up test logging with default filter (warn level).
pub fn setup_test_logging_default() {
    setup_test_logging("warn");
}

/// A test context that provides common test setup.
#[derive(Debug)]
pub struct TestContext {
    /// Temporary directory for the test.
    pub dir: TempDir,
}

impl TestContext {
    /// Create a new test context.
    #[must_use]
    pub fn new() -> Self {
        Self { dir: test_dir() }
    }

    /// Get the path to the temporary directory.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        self.dir.path()
    }

    /// Create a file in the test directory.
    #[must_use]
    pub fn create_file(&self, name: &str, content: &str) -> PathBuf {
        test_file_in_dir(&self.dir, name, content)
    }

    /// Create a subdirectory in the test directory.
    ///
    /// # Panics
    ///
    /// Panics if the directory cannot be created.
    #[must_use]
    pub fn create_subdir(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::create_dir_all(&path).expect("Failed to create subdirectory");
        path
    }
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temp_dir() {
        let dir = test_dir();
        assert!(dir.path().exists());
    }

    #[test]
    fn test_temp_file() {
        let file = test_file("hello world");
        let content = std::fs::read_to_string(file.path()).unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_temp_file_with_extension() {
        let file = test_file_with_extension("content", "toml");
        assert!(file.path().to_string_lossy().ends_with(".toml"));
    }

    #[test]
    fn test_file_in_dir_helper() {
        let dir = super::test_dir();
        let path = super::test_file_in_dir(&dir, "subdir/test.txt", "content");

        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "content");
    }

    #[test]
    fn test_context() {
        let ctx = TestContext::new();
        assert!(ctx.path().exists());

        let file_path = ctx.create_file("test.txt", "hello");
        assert!(file_path.exists());

        let subdir = ctx.create_subdir("mydir");
        assert!(subdir.is_dir());
    }
}
