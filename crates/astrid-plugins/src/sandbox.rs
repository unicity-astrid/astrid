//! OS-level sandbox profiles for plugin MCP server processes.
//!
//! Generates platform-specific sandbox configurations that constrain
//! what a plugin's child process can access:
//!
//! - **Linux**: Landlock (kernel 5.13+, network restrictions require ABI v5 / kernel 6.7+)
//! - **macOS**: `sandbox-exec` with Scheme DSL profiles (deprecated but functional)
//! - **Other**: No-op with a warning
//!
//! These profiles are applied to the `tokio::process::Command` before
//! spawning the MCP server process.

use std::path::{Path, PathBuf};

#[cfg(not(target_os = "macos"))]
use tracing::warn;

#[cfg(target_os = "macos")]
use crate::error::PluginError;
use crate::error::PluginResult;

/// Resource limits applied to Tier 2 (Node.js) plugin subprocesses.
///
/// Enforced via `setrlimit` in a `pre_exec` hook on Linux. Not yet
/// enforced on macOS (sandbox-exec does not support resource limits).
///
/// **Important**: `RLIMIT_NPROC` is per-UID on Linux, not per-process.
/// The limit must account for all processes the user may be running.
/// `RLIMIT_AS` limits virtual address space (not RSS) — Node.js/V8
/// routinely reserves 1-2 GB of virtual address space at startup, so
/// this must be set high enough for V8's memory management.
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// Maximum number of processes/threads (`RLIMIT_NPROC`).
    /// Note: this is per-UID on Linux, not per-process tree.
    pub max_processes: u64,
    /// Maximum virtual address space in bytes (`RLIMIT_AS`).
    /// Set high enough for V8's virtual memory reservations (~4 GB).
    pub max_memory_bytes: u64,
    /// Maximum number of open file descriptors (`RLIMIT_NOFILE`).
    pub max_open_files: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_processes: 256,
            max_memory_bytes: 4 * 1024 * 1024 * 1024, // 4 GB virtual address space
            max_open_files: 256,
        }
    }
}

/// Sandbox profile for constraining a plugin MCP server process.
///
/// The profile grants:
/// - Read+write access to `workspace_root`
/// - Read-only access to `plugin_dir` and system library paths
/// - Network access restricted to `allowed_network` hosts (best-effort)
/// - Optional resource limits for Tier 2 subprocesses
#[derive(Debug, Clone)]
pub struct SandboxProfile {
    /// Workspace root — plugin gets read+write access.
    pub workspace_root: PathBuf,
    /// Plugin directory — read-only access for loading plugin files.
    pub plugin_dir: PathBuf,
    /// Additional paths the plugin may read (e.g. config dirs).
    pub extra_read_paths: Vec<PathBuf>,
    /// Additional paths the plugin may read+write (e.g. data dirs).
    pub extra_write_paths: Vec<PathBuf>,
    /// Allowed network destinations (host or host:port patterns).
    /// Empty means no network restrictions are applied.
    pub allowed_network: Vec<String>,
    /// Optional resource limits for subprocess confinement.
    pub resource_limits: Option<ResourceLimits>,
    /// Optional HOME override for the subprocess environment.
    pub home_override: Option<PathBuf>,
}

impl SandboxProfile {
    /// Create a new sandbox profile.
    #[must_use]
    pub fn new(workspace_root: PathBuf, plugin_dir: PathBuf) -> Self {
        Self {
            workspace_root,
            plugin_dir,
            extra_read_paths: Vec::new(),
            extra_write_paths: Vec::new(),
            allowed_network: Vec::new(),
            resource_limits: None,
            home_override: None,
        }
    }

    /// Create a sandbox profile for a Tier 2 Node.js plugin.
    ///
    /// Sets up:
    /// - Read-only access to `install_dir` (plugin code + `node_modules`)
    /// - Read-write access to `data_dir` (plugin's private data) as the `workspace_root`
    /// - `HOME` override pointing to `data_dir`
    /// - Default resource limits (`RLIMIT_NPROC=256`, `RLIMIT_AS=4GB`, `RLIMIT_NOFILE=256`)
    ///
    /// Note: `workspace_root` is set to `data_dir` since Tier 2 plugins have no
    /// workspace access — their writable root is their isolated data directory.
    #[must_use]
    pub fn for_node_plugin(install_dir: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            workspace_root: data_dir.clone(),
            plugin_dir: install_dir,
            extra_read_paths: Vec::new(),
            extra_write_paths: Vec::new(),
            allowed_network: Vec::new(),
            resource_limits: Some(ResourceLimits::default()),
            home_override: Some(data_dir),
        }
    }

    /// Add extra read-only paths.
    #[must_use]
    pub fn with_extra_read_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.extra_read_paths = paths;
        self
    }

    /// Add extra read+write paths.
    #[must_use]
    pub fn with_extra_write_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.extra_write_paths = paths;
        self
    }

    /// Set allowed network destinations.
    #[must_use]
    pub fn with_allowed_network(mut self, hosts: Vec<String>) -> Self {
        self.allowed_network = hosts;
        self
    }

    /// Set resource limits for the subprocess.
    #[must_use]
    pub fn with_resource_limits(mut self, limits: ResourceLimits) -> Self {
        self.resource_limits = Some(limits);
        self
    }

    /// Get the HOME override, if set.
    #[must_use]
    pub fn home_override(&self) -> Option<&Path> {
        self.home_override.as_deref()
    }

    /// Wrap a command with platform-specific sandbox enforcement.
    ///
    /// On macOS, this prepends `sandbox-exec -f <profile>` to the command.
    /// On Linux, Landlock rules are applied via environment-based setup.
    /// On unsupported platforms, returns the command unchanged with a warning.
    ///
    /// # Errors
    ///
    /// Returns an error if the sandbox profile cannot be generated.
    pub fn wrap_command(
        &self,
        command: &str,
        args: &[String],
    ) -> PluginResult<(String, Vec<String>)> {
        self.platform_wrap_command(command, args)
    }

    #[cfg(target_os = "macos")]
    fn platform_wrap_command(
        &self,
        command: &str,
        args: &[String],
    ) -> PluginResult<(String, Vec<String>)> {
        let profile_content = self.generate_macos_profile(command);

        // Write the profile to a temp file
        let profile_path =
            std::env::temp_dir().join(format!("astrid-sandbox-{}.sb", std::process::id()));
        std::fs::write(&profile_path, &profile_content).map_err(|e| {
            PluginError::SandboxError(format!("Failed to write sandbox profile: {e}"))
        })?;

        let mut sandbox_args = vec![
            "-f".to_string(),
            profile_path.to_string_lossy().to_string(),
            command.to_string(),
        ];
        sandbox_args.extend(args.iter().cloned());

        Ok(("sandbox-exec".to_string(), sandbox_args))
    }

    #[cfg(target_os = "linux")]
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    fn platform_wrap_command(
        &self,
        command: &str,
        args: &[String],
    ) -> PluginResult<(String, Vec<String>)> {
        // On Linux, Landlock is applied programmatically before exec.
        // For child processes, we set environment variables that a helper
        // wrapper can use to apply Landlock rules. In practice, the runtime
        // applies Landlock to the child process via pre_exec hooks.
        //
        // For now, return the command unchanged — Landlock integration
        // requires pre_exec hooks which are set up in McpPlugin::load().
        warn!(
            "Linux Landlock sandbox profiles are applied via pre_exec hooks, \
             not command wrapping. Command returned unchanged."
        );
        Ok((command.to_string(), args.to_vec()))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn platform_wrap_command(
        &self,
        command: &str,
        args: &[String],
    ) -> PluginResult<(String, Vec<String>)> {
        warn!(
            "OS-level sandboxing is not available on this platform. \
             Plugin process will run without sandbox restrictions."
        );
        Ok((command.to_string(), args.to_vec()))
    }

    /// Generate a macOS sandbox-exec Scheme DSL profile.
    ///
    /// `sandbox-exec` is deprecated since macOS 10.x but still functional
    /// and used by Chromium, VS Code, and other major projects.
    #[cfg(target_os = "macos")]
    fn generate_macos_profile(&self, command: &str) -> String {
        use std::fmt::Write;

        let mut profile = String::new();
        profile.push_str("(version 1)\n");
        profile.push_str("(deny default)\n\n");

        // Allow reading plugin dir (plugin code + dependencies)
        let _ = writeln!(
            profile,
            "(allow file-read* (subpath \"{}\"))",
            self.plugin_dir.display()
        );

        // Allow read+write to workspace root
        let _ = writeln!(
            profile,
            "(allow file-read* (subpath \"{}\"))",
            self.workspace_root.display()
        );
        let _ = writeln!(
            profile,
            "(allow file-write* (subpath \"{}\"))",
            self.workspace_root.display()
        );

        // System library paths (node, shared libs)
        for sys_path in &[
            "/usr/lib",
            "/usr/local/lib",
            "/usr/local/bin",
            "/usr/bin",
            "/opt/homebrew",
            "/private/var/folders", // temp dirs
        ] {
            let _ = writeln!(profile, "(allow file-read* (subpath \"{sys_path}\"))");
        }

        // Extra read paths
        for path in &self.extra_read_paths {
            let _ = writeln!(
                profile,
                "(allow file-read* (subpath \"{}\"))",
                path.display()
            );
        }

        // Extra write paths (e.g. plugin data dirs)
        for path in &self.extra_write_paths {
            let _ = writeln!(
                profile,
                "(allow file-read* (subpath \"{}\"))",
                path.display()
            );
            let _ = writeln!(
                profile,
                "(allow file-write* (subpath \"{}\"))",
                path.display()
            );
        }

        // Allow process execution for the command and node runtime
        let _ = writeln!(profile, "(allow process-exec (literal \"{command}\"))");
        if let Ok(node_path) = which::which("node") {
            let _ = writeln!(
                profile,
                "(allow process-exec (literal \"{}\"))",
                node_path.display()
            );
        }
        profile.push_str("(allow process-fork)\n");

        // Allow sysctl and mach lookups (required for process execution)
        profile.push_str("(allow sysctl-read)\n");
        profile.push_str("(allow mach-lookup)\n");

        // Network access
        if self.allowed_network.is_empty() {
            // No restrictions specified — allow all outbound
            profile.push_str("(allow network-outbound)\n");
            profile.push_str("(allow network-inbound)\n");
        } else {
            // Allow loopback (required for stdio transport)
            profile.push_str("(allow network-outbound (local ip \"localhost:*\"))\n");
            for host in &self.allowed_network {
                let _ = writeln!(profile, "(allow network-outbound (remote ip \"{host}:*\"))");
            }
        }

        profile
    }

    /// Get the Landlock rule specifications for Linux.
    ///
    /// Returns a list of `(path, access_flags)` tuples suitable for
    /// Landlock `PathBeneath` rules. Caller is responsible for applying
    /// these via Landlock ABI.
    #[cfg(target_os = "linux")]
    #[must_use]
    pub fn landlock_rules(&self) -> Vec<LandlockPathRule> {
        use std::path::Path;
        let mut rules = Vec::new();

        // Workspace: read + write
        rules.push(LandlockPathRule {
            path: self.workspace_root.clone(),
            read: true,
            write: true,
        });

        // Plugin dir: read only
        rules.push(LandlockPathRule {
            path: self.plugin_dir.clone(),
            read: true,
            write: false,
        });

        // System paths: read only
        for sys_path in &[
            Path::new("/usr/lib"),
            Path::new("/usr/local/lib"),
            Path::new("/usr/bin"),
            Path::new("/usr/local/bin"),
        ] {
            if sys_path.exists() {
                rules.push(LandlockPathRule {
                    path: sys_path.to_path_buf(),
                    read: true,
                    write: false,
                });
            }
        }

        // Extra read paths
        for path in &self.extra_read_paths {
            rules.push(LandlockPathRule {
                path: path.clone(),
                read: true,
                write: false,
            });
        }

        // Extra write paths (e.g. plugin data dirs)
        for path in &self.extra_write_paths {
            rules.push(LandlockPathRule {
                path: path.clone(),
                read: true,
                write: true,
            });
        }

        rules
    }
}

/// A Landlock path rule specification.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub struct LandlockPathRule {
    /// Filesystem path.
    pub path: PathBuf,
    /// Allow read access.
    pub read: bool,
    /// Allow write access.
    pub write: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_profile_creation() {
        let profile = SandboxProfile::new(
            PathBuf::from("/workspace"),
            PathBuf::from("/plugins/my-plugin"),
        );
        assert_eq!(profile.workspace_root, PathBuf::from("/workspace"));
        assert_eq!(profile.plugin_dir, PathBuf::from("/plugins/my-plugin"));
        assert!(profile.extra_read_paths.is_empty());
        assert!(profile.allowed_network.is_empty());
    }

    #[test]
    fn test_sandbox_profile_builder() {
        let profile = SandboxProfile::new(
            PathBuf::from("/workspace"),
            PathBuf::from("/plugins/my-plugin"),
        )
        .with_extra_read_paths(vec![PathBuf::from("/etc/ssl")])
        .with_allowed_network(vec!["api.github.com".to_string()]);

        assert_eq!(profile.extra_read_paths.len(), 1);
        assert_eq!(profile.allowed_network.len(), 1);
    }

    #[test]
    fn test_wrap_command_returns_valid_output() {
        let profile = SandboxProfile::new(
            PathBuf::from("/workspace"),
            PathBuf::from("/plugins/my-plugin"),
        );
        let (cmd, args) = profile
            .wrap_command("node", &["dist/index.js".to_string()])
            .unwrap();

        // On any platform, should return some command + args
        assert!(!cmd.is_empty());
        assert!(!args.is_empty() || cmd == "node");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_profile_generation() {
        let profile = SandboxProfile::new(
            PathBuf::from("/workspace"),
            PathBuf::from("/plugins/my-plugin"),
        )
        .with_allowed_network(vec!["api.github.com".to_string()]);

        let content = profile.generate_macos_profile("node");
        assert!(content.contains("(version 1)"));
        assert!(content.contains("(deny default)"));
        assert!(content.contains("/workspace"));
        assert!(content.contains("/plugins/my-plugin"));
        assert!(content.contains("api.github.com"));
    }

    #[test]
    fn test_for_node_plugin() {
        let profile = SandboxProfile::for_node_plugin(
            PathBuf::from("/install/my-plugin"),
            PathBuf::from("/data/my-plugin"),
        );
        assert_eq!(profile.workspace_root, PathBuf::from("/data/my-plugin"));
        assert_eq!(profile.plugin_dir, PathBuf::from("/install/my-plugin"));
        assert!(profile.extra_write_paths.is_empty());
        assert!(profile.resource_limits.is_some());
        assert_eq!(
            profile.home_override,
            Some(PathBuf::from("/data/my-plugin"))
        );
    }

    #[test]
    fn test_resource_limits_defaults() {
        let limits = ResourceLimits::default();
        assert_eq!(limits.max_processes, 256);
        assert_eq!(limits.max_memory_bytes, 4 * 1024 * 1024 * 1024);
        assert_eq!(limits.max_open_files, 256);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_landlock_rules() {
        let profile = SandboxProfile::new(
            PathBuf::from("/workspace"),
            PathBuf::from("/plugins/my-plugin"),
        )
        .with_extra_read_paths(vec![PathBuf::from("/etc/ssl")]);

        let rules = profile.landlock_rules();
        // At minimum: workspace (rw) + plugin dir (ro) + extra (ro)
        assert!(rules.len() >= 3);

        // Workspace should be read+write
        let ws_rule = rules.iter().find(|r| r.path == PathBuf::from("/workspace"));
        assert!(ws_rule.is_some());
        assert!(ws_rule.unwrap().read);
        assert!(ws_rule.unwrap().write);

        // Plugin dir should be read-only
        let pd_rule = rules
            .iter()
            .find(|r| r.path == PathBuf::from("/plugins/my-plugin"));
        assert!(pd_rule.is_some());
        assert!(pd_rule.unwrap().read);
        assert!(!pd_rule.unwrap().write);
    }
}
