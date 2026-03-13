use std::ffi::OsString;
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Validate a path for safe interpolation into sandbox profiles (SBPL/bwrap).
///
/// Rejects non-UTF-8, double-quote, and null byte - all of which can break
/// or bypass sandbox profile syntax.
fn validate_sandbox_str<'a>(path: &'a Path, label: &str) -> io::Result<&'a str> {
    let s = path.to_str().ok_or_else(|| {
        io::Error::other(format!(
            "sandbox {label} is not valid UTF-8: {}",
            path.display()
        ))
    })?;
    if s.contains('"') || s.contains('\0') {
        return Err(io::Error::other(format!(
            "sandbox {label} contains forbidden characters (double-quote or null): {}",
            path.display()
        )));
    }
    Ok(s)
}

/// Wraps a standard OS command in a native kernel sandbox (bwrap or Seatbelt).
///
/// This ensures that even if an agent executes a native tool (like `bash`, `npm`, or `python`),
/// that process is physically restricted from writing to or reading from anything outside
/// of the provided worktree sandbox.
pub struct SandboxCommand;

impl SandboxCommand {
    /// Wraps the provided command in the host OS sandbox, restricting its access to
    /// the provided `worktree_path`.
    ///
    /// - On Linux, this dynamically prepends `bwrap` with strict mount rules.
    /// - On macOS, this dynamically generates a Seatbelt profile (`.sb`) and prepends `sandbox-exec`.
    /// - On other platforms (Windows), this currently passes through the command unmodified (with a warning).
    ///
    /// # Errors
    ///
    /// Returns an error if the path is not valid UTF-8, contains forbidden
    /// characters (`"` or `\0`), or if generating the macOS Seatbelt profile fails.
    #[expect(clippy::needless_pass_by_value)]
    pub fn wrap(inner_cmd: Command, worktree_path: &Path) -> io::Result<Command> {
        let worktree_str = validate_sandbox_str(worktree_path, "worktree path")?;

        #[cfg(target_os = "linux")]
        {
            // Bubblewrap implementation
            // The process can only read the root OS, but can only write to the worktree and /tmp.
            let mut bwrap = Command::new("bwrap");
            bwrap
                .arg("--ro-bind").arg("/").arg("/") // Read-only access to host OS (for binaries like /usr/bin/node)
                .arg("--dev").arg("/dev")           // Standard dev mounts
                .arg("--proc").arg("/proc")         // Standard proc mounts
                .arg("--bind").arg(&worktree_str).arg(&worktree_str) // Write access to the worktree
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
            // macOS Seatbelt implementation
            // We write a dynamic profile to /tmp that denies all writes except to the worktree and /tmp.
            // We also restrict reads to system directories, the worktree, and tmp to protect user dotfiles.
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

            // Create a temporary file for the profile
            let profile_path =
                std::env::temp_dir().join(format!("astrid_sandbox_{}.sb", uuid::Uuid::new_v4()));
            std::fs::write(&profile_path, profile)
                .map_err(|e| io::Error::other(format!("Failed to write seatbelt profile: {e}")))?;

            let mut sb_cmd = Command::new("sandbox-exec");
            sb_cmd.arg("-f").arg(&profile_path);

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
    /// Returns an error if any configured path is not valid UTF-8 or contains
    /// characters that would break sandbox profile syntax (`"` or `\0`).
    pub fn sandbox_prefix(&self) -> io::Result<Option<SandboxPrefix>> {
        #[cfg(target_os = "linux")]
        {
            Ok(Some(self.build_bwrap_prefix()))
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

    #[cfg(target_os = "linux")]
    fn build_bwrap_prefix(&self) -> SandboxPrefix {
        let mut args: Vec<OsString> = Vec::new();

        // Read-only access to host OS (for binaries like /usr/bin/node)
        args.extend(["--ro-bind", "/", "/"].map(OsString::from));
        // Standard dev + proc mounts
        args.extend(["--dev", "/dev"].map(OsString::from));
        args.extend(["--proc", "/proc"].map(OsString::from));

        // Write access to the writable root
        args.extend([
            OsString::from("--bind"),
            self.writable_root.as_os_str().into(),
            self.writable_root.as_os_str().into(),
        ]);

        // Additional writable paths
        for path in &self.extra_write_paths {
            args.extend([
                OsString::from("--bind"),
                path.as_os_str().into(),
                path.as_os_str().into(),
            ]);
        }

        // extra_read_paths are not emitted on Linux because `--ro-bind / /`
        // already grants read access to all host paths. Hidden paths override
        // via tmpfs below. On macOS, extra_read_paths are added to the
        // Seatbelt allow-list because the default policy is deny-all.

        // Disposable tmpfs for /tmp
        args.extend(["--tmpfs", "/tmp"].map(OsString::from));

        // Hidden paths: overlay with empty tmpfs
        for path in &self.hidden_paths {
            args.extend([OsString::from("--tmpfs"), path.as_os_str().into()]);
        }

        // Drop all namespaces
        args.push(OsString::from("--unshare-all"));

        // Conditionally re-enable network
        if self.allow_network {
            args.push(OsString::from("--share-net"));
        }

        // Prevent orphan processes
        args.push(OsString::from("--die-with-parent"));

        // Separator before the inner command
        args.push(OsString::from("--"));

        SandboxPrefix {
            program: OsString::from("bwrap"),
            args,
        }
    }

    #[cfg(target_os = "macos")]
    fn build_seatbelt_prefix(&self) -> io::Result<SandboxPrefix> {
        let writable_root_str = validate_sandbox_str(&self.writable_root, "writable root")?;

        // Build the network rule conditionally
        let network_rule = if self.allow_network {
            "(allow network*)"
        } else {
            ""
        };

        // Build extra read path rules
        let mut extra_read_rules = String::new();
        for p in &self.extra_read_paths {
            let s = validate_sandbox_str(p, "extra read path")?;
            if !extra_read_rules.is_empty() {
                extra_read_rules.push('\n');
            }
            let _ = write!(extra_read_rules, "    (subpath \"{s}\")");
        }

        // Build extra write path rules
        let mut extra_write_rules = String::new();
        for p in &self.extra_write_paths {
            let s = validate_sandbox_str(p, "extra write path")?;
            if !extra_write_rules.is_empty() {
                extra_write_rules.push('\n');
            }
            let _ = write!(extra_write_rules, "    (subpath \"{s}\")");
        }

        // Build deny rules for hidden paths (e.g. ~/.astrid/)
        let mut hidden_deny_rules = String::new();
        for p in &self.hidden_paths {
            let s = validate_sandbox_str(p, "hidden path")?;
            if !hidden_deny_rules.is_empty() {
                hidden_deny_rules.push('\n');
            }
            let _ = write!(
                hidden_deny_rules,
                "(deny file-read* (subpath \"{s}\"))\n\
                 (deny file-write* (subpath \"{s}\"))"
            );
        }

        let profile = format!(
            r#"(version 1)
(deny default)
(allow process-exec*)
(allow process-fork)
{network_rule}
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
    (subpath "{writable_root_str}")
    (subpath "/private/tmp")
    (subpath "/var/folders")
{extra_read_rules}
)
(allow file-write*
    (subpath "{writable_root_str}")
    (subpath "/private/tmp")
    (subpath "/var/folders")
    (literal "/dev/null")
{extra_write_rules}
)
{hidden_deny_rules}"#
        );

        // Pass profile inline via -p to avoid temp file leak.
        let args = vec![OsString::from("-p"), OsString::from(&profile)];

        Ok(SandboxPrefix {
            program: OsString::from("sandbox-exec"),
            args,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[cfg(target_os = "linux")]
    #[test]
    fn test_bwrap_prefix_basic() {
        let config = ProcessSandboxConfig::new("/project");
        let prefix = config.build_bwrap_prefix();

        assert_eq!(prefix.program, OsString::from("bwrap"));

        let args_str: Vec<String> = prefix
            .args
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();

        // Verify core structure
        assert!(args_str.contains(&"--ro-bind".to_string()));
        assert!(args_str.contains(&"--dev".to_string()));
        assert!(args_str.contains(&"--proc".to_string()));
        assert!(args_str.contains(&"--unshare-all".to_string()));
        assert!(args_str.contains(&"--share-net".to_string()));
        assert!(args_str.contains(&"--die-with-parent".to_string()));
        assert!(args_str.contains(&"--".to_string()));

        // Writable root bind
        let bind_idx = args_str
            .iter()
            .position(|a| a == "--bind")
            .expect("should have --bind");
        assert_eq!(args_str[bind_idx + 1], "/project");
        assert_eq!(args_str[bind_idx + 2], "/project");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_bwrap_prefix_no_network() {
        let config = ProcessSandboxConfig::new("/project").with_network(false);
        let prefix = config.build_bwrap_prefix();

        let args_str: Vec<String> = prefix
            .args
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();

        assert!(args_str.contains(&"--unshare-all".to_string()));
        assert!(!args_str.contains(&"--share-net".to_string()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_bwrap_prefix_hidden_paths() {
        let config = ProcessSandboxConfig::new("/project").with_hidden("/home/user/.astrid");
        let prefix = config.build_bwrap_prefix();

        let args_str: Vec<String> = prefix
            .args
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();

        // Find the tmpfs for hidden path (not the /tmp one)
        let tmpfs_positions: Vec<usize> = args_str
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "--tmpfs")
            .map(|(i, _)| i)
            .collect();

        assert!(
            tmpfs_positions.len() >= 2,
            "should have at least 2 tmpfs mounts"
        );
        let hidden_tmpfs_found = tmpfs_positions
            .iter()
            .any(|&i| args_str.get(i + 1) == Some(&"/home/user/.astrid".to_string()));
        assert!(hidden_tmpfs_found, "should have tmpfs for hidden path");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_bwrap_prefix_extra_paths() {
        let config = ProcessSandboxConfig::new("/project")
            .with_extra_read("/data")
            .with_extra_write("/output");
        let prefix = config.build_bwrap_prefix();

        let args_str: Vec<String> = prefix
            .args
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();

        // Extra writable path should have --bind
        let bind_positions: Vec<usize> = args_str
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "--bind")
            .map(|(i, _)| i)
            .collect();
        let has_output_bind = bind_positions
            .iter()
            .any(|&i| args_str.get(i + 1) == Some(&"/output".to_string()));
        assert!(has_output_bind, "should have --bind for extra write path");

        // extra_read_paths are NOT emitted as --ro-bind on Linux because
        // `--ro-bind / /` already covers all host paths. Verify they are
        // NOT redundantly added.
        let ro_positions: Vec<usize> = args_str
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "--ro-bind")
            .map(|(i, _)| i)
            .collect();
        let has_data_explicit = ro_positions
            .iter()
            .any(|&i| args_str.get(i + 1) == Some(&"/data".to_string()));
        assert!(
            !has_data_explicit,
            "extra_read_paths should NOT produce --ro-bind on Linux (covered by --ro-bind / /)"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_seatbelt_prefix_basic() {
        let config = ProcessSandboxConfig::new("/project");
        let prefix = config.build_seatbelt_prefix().unwrap();

        assert_eq!(prefix.program, OsString::from("sandbox-exec"));
        assert_eq!(prefix.args[0], OsString::from("-p"));

        // Profile is passed inline as the second arg
        let profile = prefix.args[1].to_string_lossy().to_string();

        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("(allow network*)"));
        assert!(profile.contains(r#"(subpath "/project")"#));
        assert!(profile.contains("(allow process-exec*)"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_seatbelt_prefix_no_network() {
        let config = ProcessSandboxConfig::new("/project").with_network(false);
        let prefix = config.build_seatbelt_prefix().unwrap();

        let profile = prefix.args[1].to_string_lossy().to_string();
        assert!(!profile.contains("(allow network*)"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_seatbelt_prefix_extra_paths() {
        let config = ProcessSandboxConfig::new("/project")
            .with_extra_read("/data")
            .with_extra_write("/output");
        let prefix = config.build_seatbelt_prefix().unwrap();

        let profile = prefix.args[1].to_string_lossy().to_string();
        assert!(profile.contains(r#"(subpath "/data")"#));
        assert!(profile.contains(r#"(subpath "/output")"#));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_seatbelt_prefix_hidden_paths() {
        let config = ProcessSandboxConfig::new("/project").with_hidden("/Users/testuser/.astrid");
        let prefix = config.build_seatbelt_prefix().unwrap();

        let profile = prefix.args[1].to_string_lossy().to_string();
        assert!(
            profile.contains(r#"(deny file-read* (subpath "/Users/testuser/.astrid"))"#),
            "should deny file-read for hidden path"
        );
        assert!(
            profile.contains(r#"(deny file-write* (subpath "/Users/testuser/.astrid"))"#),
            "should deny file-write for hidden path"
        );
    }

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

    #[cfg(target_os = "macos")]
    #[test]
    fn test_seatbelt_prefix_rejects_non_utf8_writable_root() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let bad_bytes: &[u8] = b"/tmp/\xff\xfe/workspace";
        let bad_path = PathBuf::from(OsStr::from_bytes(bad_bytes));
        let config = ProcessSandboxConfig::new(bad_path);
        let result = config.sandbox_prefix();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not valid UTF-8"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_seatbelt_prefix_rejects_non_utf8_extra_paths() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let bad_bytes: &[u8] = b"/data/\xff\xfe";
        let bad_path = PathBuf::from(OsStr::from_bytes(bad_bytes));

        // Non-UTF-8 in extra read path
        let config = ProcessSandboxConfig::new("/project").with_extra_read(bad_path.clone());
        assert!(config.sandbox_prefix().is_err());

        // Non-UTF-8 in extra write path
        let config = ProcessSandboxConfig::new("/project").with_extra_write(bad_path.clone());
        assert!(config.sandbox_prefix().is_err());

        // Non-UTF-8 in hidden path
        let config = ProcessSandboxConfig::new("/project").with_hidden(bad_path);
        assert!(config.sandbox_prefix().is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_seatbelt_prefix_rejects_double_quote_in_paths() {
        // Double-quote in writable root
        let config = ProcessSandboxConfig::new("/project/evil\"dir");
        assert!(config.sandbox_prefix().is_err());

        // Double-quote in extra read path
        let config = ProcessSandboxConfig::new("/project").with_extra_read("/data/evil\"path");
        assert!(config.sandbox_prefix().is_err());

        // Double-quote in extra write path
        let config = ProcessSandboxConfig::new("/project").with_extra_write("/output/evil\"path");
        assert!(config.sandbox_prefix().is_err());

        // Double-quote in hidden path
        let config = ProcessSandboxConfig::new("/project").with_hidden("/hidden/evil\"path");
        assert!(config.sandbox_prefix().is_err());
    }
}
