use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "linux")]
mod bwrap;
#[cfg(target_os = "macos")]
mod seatbelt;

/// Validate a path for safe interpolation into sandbox profiles (SBPL/bwrap).
///
/// Rejects relative paths, non-UTF-8, double-quote, backslash, and null byte -
/// all of which can break or bypass sandbox profile syntax.
fn validate_sandbox_str<'a>(path: &'a Path, label: &str) -> io::Result<&'a str> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "sandbox {label} must be an absolute path, got: {}",
                path.display()
            ),
        ));
    }
    let s = path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("sandbox {label} is not valid UTF-8: {}", path.display()),
        )
    })?;
    if s.contains(['"', '\\', '\0']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "sandbox {label} contains forbidden characters (double-quote, backslash, or null): {}",
                path.display()
            ),
        ));
    }
    Ok(s)
}

/// Wraps a standard OS command in a native kernel sandbox (bwrap or Seatbelt).
///
/// Ensures that agent-executed native tools are restricted from accessing
/// anything outside the provided worktree sandbox.
pub struct SandboxCommand;

impl SandboxCommand {
    /// Wraps the provided command in the host OS sandbox, restricting its access to
    /// the provided `worktree_path`.
    ///
    /// - On Linux, this dynamically prepends `bwrap` with strict mount rules.
    /// - On macOS, this dynamically generates a Seatbelt profile and prepends `sandbox-exec -p`.
    /// - On other platforms (Windows), this currently passes through the command unmodified (with a warning).
    ///
    /// # Errors
    ///
    /// Returns an error if the worktree path is not absolute, not valid UTF-8,
    /// or contains characters unsafe for SBPL interpolation (double-quote,
    /// backslash, or null byte).
    ///
    /// # Panics
    ///
    /// Panics on macOS if `validate_sandbox_str` passes but the path is not
    /// valid UTF-8. This is unreachable because the validation rejects
    /// non-UTF-8 paths.
    #[allow(clippy::needless_pass_by_value)] // Consumed on macOS early return, borrowed on Linux bwrap
    pub fn wrap(inner_cmd: Command, worktree_path: &Path) -> io::Result<Command> {
        // Validate on all platforms for defense in depth and API consistency.
        // On macOS the validated string is needed for SBPL interpolation.
        // On Linux bwrap passes paths as argv entries (no injection risk),
        // but we still reject unsafe paths at the API boundary.
        let _ = validate_sandbox_str(worktree_path, "worktree path")?;

        #[cfg(target_os = "linux")]
        {
            // Bubblewrap implementation - paths are passed as separate argv entries (no injection).
            // The process can only read the root OS, but can only write to the worktree and /tmp.
            let mut bwrap = Command::new("bwrap");
            bwrap
                .arg("--ro-bind").arg("/").arg("/") // Read-only access to host OS (for binaries like /usr/bin/node)
                .arg("--dev").arg("/dev")           // Standard dev mounts
                .arg("--proc").arg("/proc")         // Standard proc mounts
                .arg("--bind").arg(worktree_path).arg(worktree_path) // Write access to the worktree
                .arg("--tmpfs").arg("/tmp")         // Disposable tmpfs
                .arg("--unshare-all")               // Drop namespaces (network, pid, etc.)
                .arg("--share-net")                 // Re-enable network so npm/cargo can fetch
                .arg("--die-with-parent"); // Prevent orphan processes

            // Extract the original command and args, and append them to bwrap
            bwrap.arg(inner_cmd.get_program());
            for arg in inner_cmd.get_args() {
                bwrap.arg(arg);
            }

            // Inherit the env and current_dir from the original command
            for (k, v) in inner_cmd.get_envs() {
                if let Some(v) = v {
                    bwrap.env(k, v);
                } else {
                    bwrap.env_remove(k);
                }
            }
            if let Some(dir) = inner_cmd.get_current_dir() {
                bwrap.current_dir(dir);
            }

            Ok(bwrap)
        }

        #[cfg(target_os = "macos")]
        {
            // sandbox-exec (Seatbelt) is deprecated on macOS 15+ (Darwin >= 24).
            if seatbelt::darwin_major_version() >= 24 {
                tracing::warn!(
                    "macOS 15+ detected: sandbox-exec is deprecated. Running host process unsandboxed."
                );
                return Ok(inner_cmd);
            }

            // Safe: validate_sandbox_str above confirmed valid UTF-8.
            let worktree_str = worktree_path
                .to_str()
                .expect("unreachable: validated UTF-8 above");

            // macOS Seatbelt implementation
            // Deny all writes except to the worktree and /tmp.
            // Restrict reads to system directories, the worktree, and tmp to protect user dotfiles.
            let profile = format!(
                r#"(version 1)
(deny default)
(allow process-exec*)
(allow process-fork)
(allow network*)
(allow sysctl-read)
(allow ipc-posix-shm)
(allow file-read*
    (subpath "/usr")
    (subpath "/bin")
    (subpath "/sbin")
    (subpath "/System")
    (subpath "/Library")
    (subpath "/opt")
    (subpath "/dev")
    (subpath "{worktree_str}")
    (subpath "/private/tmp")
    (subpath "/var/folders")
)
(allow file-write*
    (subpath "{worktree_str}")
    (subpath "/private/tmp")
    (subpath "/var/folders")
    (literal "/dev/null")
)"#
            );

            // Pass profile inline via -p to avoid temp-file leaks and TOCTOU races.
            let mut sb_cmd = Command::new("sandbox-exec");
            sb_cmd.arg("-p").arg(&profile);

            // Extract original
            sb_cmd.arg(inner_cmd.get_program());
            for arg in inner_cmd.get_args() {
                sb_cmd.arg(arg);
            }

            // Inherit env and dir
            for (k, v) in inner_cmd.get_envs() {
                if let Some(v) = v {
                    sb_cmd.env(k, v);
                } else {
                    sb_cmd.env_remove(k);
                }
            }
            if let Some(dir) = inner_cmd.get_current_dir() {
                sb_cmd.current_dir(dir);
            }

            Ok(sb_cmd)
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            tracing::warn!(
                "Host-level sandboxing is not supported on this OS. Processes will run unsandboxed."
            );
            Ok(inner_cmd)
        }
    }
}

/// The sandbox wrapper program and its argument prefix.
///
/// The caller appends the original program and its arguments after these args.
#[derive(Debug, Clone)]
pub struct SandboxPrefix {
    /// The sandbox wrapper program (e.g., `bwrap` or `sandbox-exec`).
    pub program: OsString,
    /// Arguments to the sandbox wrapper, NOT including the inner command.
    pub args: Vec<OsString>,
}

/// Data-oriented sandbox configuration that produces a wrapper program + args
/// prefix rather than wrapping a `std::process::Command` directly.
///
/// This is useful when the consumer needs a different `Command` type (e.g.,
/// `tokio::process::Command`) but still wants OS-level sandbox wrapping.
///
/// # Example
///
/// ```rust,ignore
/// let config = ProcessSandboxConfig::new("/home/user/project")
///     .with_network(true)
///     .with_hidden("/home/user/.astrid");
///
/// if let Some(prefix) = config.sandbox_prefix()? {
///     let mut cmd = tokio::process::Command::new(&prefix.program);
///     cmd.args(&prefix.args);
///     cmd.arg("npx").args(["@anthropics/mcp-server-filesystem", "/tmp"]);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ProcessSandboxConfig {
    /// Root directory the sandboxed process can write to.
    writable_root: PathBuf,
    /// Additional read-only paths beyond the OS defaults.
    extra_read_paths: Vec<PathBuf>,
    /// Additional writable paths beyond `writable_root`.
    extra_write_paths: Vec<PathBuf>,
    /// Whether to allow network access.
    allow_network: bool,
    /// Paths to overlay with empty tmpfs (Linux) or exclude (macOS), blocking access.
    hidden_paths: Vec<PathBuf>,
}

impl ProcessSandboxConfig {
    /// Create a new sandbox config with the given writable root.
    #[must_use]
    pub fn new(writable_root: impl Into<PathBuf>) -> Self {
        Self {
            writable_root: writable_root.into(),
            extra_read_paths: Vec::new(),
            extra_write_paths: Vec::new(),
            allow_network: true,
            hidden_paths: Vec::new(),
        }
    }

    /// Set whether network access is allowed.
    #[must_use]
    pub fn with_network(mut self, allow: bool) -> Self {
        self.allow_network = allow;
        self
    }

    /// Add an additional read-only path.
    #[must_use]
    pub fn with_extra_read(mut self, path: impl Into<PathBuf>) -> Self {
        self.extra_read_paths.push(path.into());
        self
    }

    /// Add an additional writable path.
    #[must_use]
    pub fn with_extra_write(mut self, path: impl Into<PathBuf>) -> Self {
        self.extra_write_paths.push(path.into());
        self
    }

    /// Add a path to hide from the sandboxed process.
    ///
    /// On Linux, this overlays an empty tmpfs. On macOS, the path is
    /// excluded from the Seatbelt read allowlist.
    #[must_use]
    pub fn with_hidden(mut self, path: impl Into<PathBuf>) -> Self {
        self.hidden_paths.push(path.into());
        self
    }

    /// Build the sandbox wrapper prefix for this configuration.
    ///
    /// Returns `Some(prefix)` on supported platforms (Linux, macOS), `None` on
    /// unsupported platforms (e.g., Windows).
    ///
    /// # Errors
    ///
    /// Returns an error if any configured path is not valid UTF-8, not absolute,
    /// or contains characters that would break sandbox profile syntax
    /// (double-quote, backslash, or null byte).
    pub fn sandbox_prefix(&self) -> io::Result<Option<SandboxPrefix>> {
        // Validate all configured paths up front, regardless of platform.
        // This ensures the doc contract ("returns Err for non-UTF-8 or
        // forbidden chars") holds on every OS, not just macOS where SBPL
        // interpolation makes it exploitable.
        self.validate_all_paths()?;

        #[cfg(target_os = "linux")]
        {
            if bwrap::bwrap_available() {
                Ok(Some(self.build_bwrap_prefix()))
            } else {
                Ok(None)
            }
        }

        #[cfg(target_os = "macos")]
        {
            self.build_seatbelt_prefix().map(Some)
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            tracing::warn!(
                "Host-level sandboxing is not supported on this OS. \
                 MCP server will run unsandboxed."
            );
            Ok(None)
        }
    }

    /// Validate all configured paths for safe use in sandbox profiles.
    fn validate_all_paths(&self) -> io::Result<()> {
        validate_sandbox_str(&self.writable_root, "writable root")?;
        for p in &self.extra_read_paths {
            validate_sandbox_str(p, "extra read path")?;
        }
        for p in &self.extra_write_paths {
            validate_sandbox_str(p, "extra write path")?;
        }
        for p in &self.hidden_paths {
            validate_sandbox_str(p, "hidden path")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Validates that a path is safe for interpolation into an SBPL profile string.
    fn validate_sandbox_path(path: &Path) -> io::Result<()> {
        let s = path.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("sandbox path is not valid UTF-8: {}", path.display()),
            )
        })?;
        if s.contains(['"', '\\', '\0']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "sandbox path contains forbidden characters (double-quote, backslash, or null): {}",
                    path.display()
                ),
            ));
        }
        Ok(())
    }

    // --- validate_sandbox_path tests ---

    #[test]
    fn validate_sandbox_path_accepts_normal_path() {
        let path = PathBuf::from("/Users/agent/workspace/project");
        assert!(validate_sandbox_path(&path).is_ok());
    }

    #[test]
    fn validate_sandbox_path_accepts_path_with_spaces() {
        let path = PathBuf::from("/Users/agent/my project/src");
        assert!(validate_sandbox_path(&path).is_ok());
    }

    #[test]
    fn validate_sandbox_path_rejects_double_quote() {
        let path = PathBuf::from("/Users/agent/work\"inject");
        let err = validate_sandbox_path(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("forbidden characters"),
            "unexpected error message: {err}"
        );
    }

    #[test]
    fn validate_sandbox_path_rejects_sbpl_injection_payload() {
        // Simulates an actual SBPL escape attempt.
        let path = PathBuf::from(r#"/tmp/evil") (allow file-write* (subpath "/"))"#);
        let err = validate_sandbox_path(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("forbidden characters"),
            "unexpected error message: {err}"
        );
    }

    #[test]
    fn validate_sandbox_path_rejects_backslash() {
        let path = PathBuf::from("/tmp/work\\nspace");
        let err = validate_sandbox_path(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("forbidden characters"),
            "unexpected error message: {err}"
        );
    }

    #[test]
    fn validate_sandbox_path_rejects_null_byte() {
        let path = PathBuf::from("/tmp/work\0space");
        let err = validate_sandbox_path(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("forbidden characters"),
            "unexpected error message: {err}"
        );
    }

    // --- SandboxCommand::wrap() tests ---

    #[test]
    fn test_wrap_rejects_non_utf8_path() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let bad_bytes: &[u8] = b"/tmp/\xff\xfe/workspace";
        let bad_path = Path::new(OsStr::from_bytes(bad_bytes));
        let cmd = Command::new("echo");
        let result = SandboxCommand::wrap(cmd, bad_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not valid UTF-8"),
            "error should mention UTF-8: {err_msg}"
        );
    }

    #[test]
    fn test_wrap_rejects_double_quote_path() {
        let bad_path = Path::new("/tmp/evil\"injection/workspace");
        let cmd = Command::new("echo");
        let result = SandboxCommand::wrap(cmd, bad_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("forbidden characters"),
            "error should mention forbidden chars: {err_msg}"
        );
    }

    #[test]
    fn test_wrap_rejects_null_byte_path() {
        let bad_path = Path::new("/tmp/evil\0null/workspace");
        let cmd = Command::new("echo");
        let result = SandboxCommand::wrap(cmd, bad_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("forbidden characters"),
            "error should mention forbidden chars: {err_msg}"
        );
    }

    #[test]
    fn test_wrap_rejects_backslash_path() {
        let bad_path = Path::new("/tmp/work\\nspace");
        let cmd = Command::new("echo");
        let result = SandboxCommand::wrap(cmd, bad_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("forbidden characters"),
            "error should mention forbidden chars: {err_msg}"
        );
    }

    #[test]
    fn test_wrap_rejects_relative_path() {
        let bad_path = Path::new("relative/workspace");
        let cmd = Command::new("echo");
        let result = SandboxCommand::wrap(cmd, bad_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("absolute path"),
            "error should mention absolute path: {err_msg}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn wrap_uses_inline_profile() {
        let cmd = Command::new("echo");
        let path = PathBuf::from("/tmp/safe-workspace");
        let wrapped = SandboxCommand::wrap(cmd, &path).unwrap();

        if super::seatbelt::darwin_major_version() >= 24 {
            assert_eq!(
                wrapped.get_program(),
                "echo",
                "on macOS 15+, command should pass through unwrapped"
            );
        } else {
            let args: Vec<_> = wrapped.get_args().collect();
            assert_eq!(args[0], "-p", "expected -p for inline profile delivery");
            let profile = args[1].to_string_lossy();
            assert!(
                profile.contains("/tmp/safe-workspace"),
                "profile should contain the worktree path"
            );
        }
    }

    // --- ProcessSandboxConfig builder tests ---

    #[test]
    fn test_sandbox_config_builder() {
        let config = ProcessSandboxConfig::new("/project")
            .with_network(false)
            .with_extra_read("/data")
            .with_extra_write("/output")
            .with_hidden("/home/user/.astrid");

        assert_eq!(config.writable_root, PathBuf::from("/project"));
        assert!(!config.allow_network);
        assert_eq!(config.extra_read_paths, vec![PathBuf::from("/data")]);
        assert_eq!(config.extra_write_paths, vec![PathBuf::from("/output")]);
        assert_eq!(
            config.hidden_paths,
            vec![PathBuf::from("/home/user/.astrid")]
        );
    }

    #[test]
    fn test_sandbox_config_defaults() {
        let config = ProcessSandboxConfig::new("/project");
        assert!(config.allow_network);
        assert!(config.extra_read_paths.is_empty());
        assert!(config.extra_write_paths.is_empty());
        assert!(config.hidden_paths.is_empty());
    }

    // --- Cross-platform sandbox_prefix() rejection tests ---

    #[test]
    fn test_sandbox_prefix_rejects_relative_writable_root() {
        let config = ProcessSandboxConfig::new("relative/project");
        assert!(config.sandbox_prefix().is_err());
    }

    #[test]
    fn test_sandbox_prefix_rejects_non_utf8_writable_root() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let bad_bytes: &[u8] = b"/tmp/\xff\xfe/workspace";
        let bad_path = PathBuf::from(OsStr::from_bytes(bad_bytes));
        let config = ProcessSandboxConfig::new(bad_path);
        let result = config.sandbox_prefix();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not valid UTF-8"));
    }

    #[test]
    fn test_sandbox_prefix_rejects_non_utf8_extra_paths() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let bad_bytes: &[u8] = b"/data/\xff\xfe";
        let bad_path = PathBuf::from(OsStr::from_bytes(bad_bytes));

        let config = ProcessSandboxConfig::new("/project").with_extra_read(bad_path.clone());
        assert!(config.sandbox_prefix().is_err());

        let config = ProcessSandboxConfig::new("/project").with_extra_write(bad_path.clone());
        assert!(config.sandbox_prefix().is_err());

        let config = ProcessSandboxConfig::new("/project").with_hidden(bad_path);
        assert!(config.sandbox_prefix().is_err());
    }

    #[test]
    fn test_sandbox_prefix_rejects_double_quote_in_paths() {
        let config = ProcessSandboxConfig::new("/project/evil\"dir");
        assert!(config.sandbox_prefix().is_err());

        let config = ProcessSandboxConfig::new("/project").with_extra_read("/data/evil\"path");
        assert!(config.sandbox_prefix().is_err());

        let config = ProcessSandboxConfig::new("/project").with_extra_write("/output/evil\"path");
        assert!(config.sandbox_prefix().is_err());

        let config = ProcessSandboxConfig::new("/project").with_hidden("/hidden/evil\"path");
        assert!(config.sandbox_prefix().is_err());
    }

    #[test]
    fn test_sandbox_prefix_rejects_backslash_in_paths() {
        let config = ProcessSandboxConfig::new("/project/evil\\dir");
        assert!(config.sandbox_prefix().is_err());

        let config = ProcessSandboxConfig::new("/project").with_extra_read("/data/evil\\path");
        assert!(config.sandbox_prefix().is_err());

        let config = ProcessSandboxConfig::new("/project").with_extra_write("/output/evil\\path");
        assert!(config.sandbox_prefix().is_err());

        let config = ProcessSandboxConfig::new("/project").with_hidden("/hidden/evil\\path");
        assert!(config.sandbox_prefix().is_err());
    }

    #[test]
    fn test_sandbox_prefix_rejects_null_byte_in_paths() {
        let config = ProcessSandboxConfig::new("/project/evil\0dir");
        assert!(config.sandbox_prefix().is_err());

        let config = ProcessSandboxConfig::new("/project").with_extra_read("/data/evil\0path");
        assert!(config.sandbox_prefix().is_err());

        let config = ProcessSandboxConfig::new("/project").with_extra_write("/output/evil\0path");
        assert!(config.sandbox_prefix().is_err());

        let config = ProcessSandboxConfig::new("/project").with_hidden("/hidden/evil\0path");
        assert!(config.sandbox_prefix().is_err());
    }
}
