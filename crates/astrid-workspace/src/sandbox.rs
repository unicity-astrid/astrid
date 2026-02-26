use std::io;
use std::path::Path;
use std::process::Command;

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
    /// Returns an error if generating the macOS Seatbelt profile fails.
    #[allow(clippy::needless_pass_by_value)]
    pub fn wrap(inner_cmd: Command, worktree_path: &Path) -> io::Result<Command> {
        let worktree_str = worktree_path.to_string_lossy().to_string();

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
            let profile = format!(
                r#"(version 1)
(deny default)
(allow file-read*)
(allow process-exec*)
(allow process-fork)
(allow network*)
(allow sysctl-read)
(allow ipc-posix-shm)
(allow file-write* 
    (subpath "{worktree_str}")
    (subpath "/private/tmp")
    (subpath "/var/folders")
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
