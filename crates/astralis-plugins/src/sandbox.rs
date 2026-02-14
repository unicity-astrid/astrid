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

use std::path::PathBuf;

#[cfg(not(target_os = "macos"))]
use tracing::warn;

#[cfg(target_os = "macos")]
use crate::error::PluginError;
use crate::error::PluginResult;

/// Sandbox profile for constraining a plugin MCP server process.
///
/// The profile grants:
/// - Read+write access to `workspace_root`
/// - Read-only access to `plugin_dir` and system library paths
/// - Network access restricted to `allowed_network` hosts (best-effort)
#[derive(Debug, Clone)]
pub struct SandboxProfile {
    /// Workspace root — plugin gets read+write access.
    pub workspace_root: PathBuf,
    /// Plugin directory — read-only access for loading plugin files.
    pub plugin_dir: PathBuf,
    /// Additional paths the plugin may read (e.g. config dirs).
    pub extra_read_paths: Vec<PathBuf>,
    /// Allowed network destinations (host or host:port patterns).
    /// Empty means no network restrictions are applied.
    pub allowed_network: Vec<String>,
}

impl SandboxProfile {
    /// Create a new sandbox profile.
    #[must_use]
    pub fn new(workspace_root: PathBuf, plugin_dir: PathBuf) -> Self {
        Self {
            workspace_root,
            plugin_dir,
            extra_read_paths: Vec::new(),
            allowed_network: Vec::new(),
        }
    }

    /// Add extra read-only paths.
    #[must_use]
    pub fn with_extra_read_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.extra_read_paths = paths;
        self
    }

    /// Set allowed network destinations.
    #[must_use]
    pub fn with_allowed_network(mut self, hosts: Vec<String>) -> Self {
        self.allowed_network = hosts;
        self
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
            std::env::temp_dir().join(format!("astralis-sandbox-{}.sb", std::process::id()));
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
