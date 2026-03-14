use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use extism::{CurrentPlugin, Error, UserData, Val};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
use astrid_workspace::SandboxCommand;

#[derive(Debug, Deserialize)]
struct ProcessRequest<'a> {
    cmd: &'a str,
    #[serde(default)]
    args: Vec<&'a str>,
}

#[derive(Debug, Serialize)]
struct ProcessResult {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

/// Tracks active child process PIDs for cancellation.
///
/// Shared between the spawn host function (registers/unregisters PIDs) and the
/// cancel listener background task (sends SIGINT/SIGKILL on cancellation).
#[derive(Debug, Default)]
pub struct ProcessTracker {
    active_pids: std::sync::Arc<Mutex<HashSet<u32>>>,
}

impl ProcessTracker {
    /// Create a new, empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a child process PID for cancellation tracking.
    pub fn register(&self, pid: u32) {
        if pid == 0 {
            return; // Guard: PID 0 means "no process" on some platforms.
        }
        self.active_pids
            .lock()
            .expect("process tracker lock poisoned")
            .insert(pid);
    }

    /// Unregister a child process PID (process has exited).
    pub fn unregister(&self, pid: u32) {
        self.active_pids
            .lock()
            .expect("process tracker lock poisoned")
            .remove(&pid);
    }

    /// Send SIGINT to all tracked processes, then SIGKILL after a grace period.
    ///
    /// On macOS, `sandbox-exec` replaces itself via `exec()`, so the tracked
    /// PID IS the real inner command. On Linux, `bwrap` forwards signals to
    /// the inner process. Known limitation: if a future sandbox wrapper forks
    /// without forwarding signals, the inner process may survive SIGINT.
    /// The SIGKILL task re-checks `active_pids` before signaling to avoid
    /// hitting reused PIDs.
    pub fn cancel_all(&self, handle: &tokio::runtime::Handle) {
        let pids: Vec<u32> = self
            .active_pids
            .lock()
            .expect("process tracker lock poisoned")
            .iter()
            .copied()
            .collect();

        if pids.is_empty() {
            return;
        }

        // SIGINT all tracked processes.
        for &pid in &pids {
            let Some(raw) = i32::try_from(pid).ok() else {
                warn!(pid, "PID overflows i32, skipping signal");
                continue;
            };
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(raw),
                nix::sys::signal::Signal::SIGINT,
            );
        }

        // Spawn a task to SIGKILL survivors after a grace period.
        let tracker = self.active_pids.clone();
        handle.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let still_active: Vec<u32> = tracker
                .lock()
                .expect("process tracker lock poisoned")
                .iter()
                .copied()
                .collect();
            for pid in still_active {
                let Some(raw) = i32::try_from(pid).ok() else {
                    continue;
                };
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(raw),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
        });
    }
}

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_spawn_host_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let req_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], util::MAX_GUEST_PAYLOAD_LEN)?;
    let req: ProcessRequest = serde_json::from_slice(&req_bytes)
        .map_err(|e| Error::msg(format!("failed to parse process request: {e}")))?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let capsule_id = state.capsule_id.as_str().to_owned();
    let handle = state.runtime_handle.clone();
    let semaphore = state.host_semaphore.clone();
    let cancel_token = state.cancel_token.clone();
    let process_tracker = state.process_tracker.clone();
    drop(state);

    if let Some(sec) = security {
        let cmd = req.cmd.to_string();
        util::bounded_block_on(&handle, &semaphore, async {
            sec.check_host_process(&capsule_id, &cmd).await
        })
        .map_err(|e| Error::msg(format!("Security Check Failed: {e}")))?;
    } else {
        return Err(Error::msg(
            "Security Check Failed: No security gate found for host_process capability.",
        ));
    }

    let mut inner_cmd = Command::new(req.cmd);
    inner_cmd.args(&req.args);

    // Strip socket-related env vars inherited from the daemon process.
    // WASM guests cannot inject env vars (ProcessRequest has no env field),
    // but the daemon's own environment is inherited by child processes.
    // With token auth, ASTRID_SOCKET_PATH alone is insufficient to connect,
    // but belt-and-suspenders. ASTRID_SESSION_TOKEN is not currently set in
    // the environment (token is on disk), but reserved for future use.
    inner_cmd.env_remove("ASTRID_SOCKET_PATH");
    inner_cmd.env_remove("ASTRID_SESSION_TOKEN");
    inner_cmd.env_remove("ASTRID_HOME");

    let sandboxed_cmd = SandboxCommand::wrap(inner_cmd, &workspace_root)
        .map_err(|e| Error::msg(format!("failed to wrap command in sandbox: {e}")))?;

    // Spawn the child process (non-blocking) so we can track its PID.
    let mut sandboxed_cmd = sandboxed_cmd;
    sandboxed_cmd.stdout(Stdio::piped());
    sandboxed_cmd.stderr(Stdio::piped());

    let child = sandboxed_cmd
        .spawn()
        .map_err(|e| Error::msg(format!("failed to spawn command: {e}")))?;

    let pid = child.id();
    process_tracker.register(pid);

    // Wait for the child on the blocking thread pool so tokio worker threads
    // remain free for the cancel listener and other async tasks.
    let output_result =
        util::bounded_block_on_cancellable(&handle, &semaphore, &cancel_token, async move {
            tokio::task::spawn_blocking(move || child.wait_with_output())
                .await
                .map_err(std::io::Error::other)
                .and_then(|r| r)
        });

    let result = match output_result {
        Some(Ok(output)) => {
            process_tracker.unregister(pid);
            ProcessResult {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                exit_code: output.status.code().unwrap_or(-1),
            }
        },
        Some(Err(e)) => {
            process_tracker.unregister(pid);
            return Err(Error::msg(format!("failed to execute command: {e}")));
        },
        None => {
            // Cancelled (capsule unloading or tool cancellation).
            // Send explicit SIGKILL before unregistering: the process may trap
            // SIGINT, and the cancel_all grace-period task only checks
            // active_pids (which we clear below). This guarantees the process
            // is dead regardless of its signal disposition.
            warn!(capsule_id, pid, "process cancelled");
            if let Ok(raw) = i32::try_from(pid) {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(raw),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
            process_tracker.unregister(pid);
            ProcessResult {
                stdout: String::new(),
                stderr: "process cancelled".to_owned(),
                exit_code: -1,
            }
        },
    };

    let result_bytes = serde_json::to_vec(&result)?;
    let mem = plugin.memory_new(&result_bytes)?;
    outputs[0] = plugin.memory_to_val(mem);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::Arc;

    #[test]
    fn tracker_register_unregister() {
        let tracker = ProcessTracker::new();
        tracker.register(1234);
        tracker.register(5678);
        assert_eq!(tracker.active_pids.lock().unwrap().len(), 2);
        tracker.unregister(1234);
        assert_eq!(tracker.active_pids.lock().unwrap().len(), 1);
        assert!(tracker.active_pids.lock().unwrap().contains(&5678));
    }

    #[test]
    fn tracker_ignores_pid_zero() {
        let tracker = ProcessTracker::new();
        tracker.register(0);
        assert!(tracker.active_pids.lock().unwrap().is_empty());
    }

    #[test]
    fn tracker_cancel_all_empty_is_noop() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let tracker = ProcessTracker::new();
        // Should not panic or error on empty tracker.
        tracker.cancel_all(rt.handle());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tracker_cancel_all_kills_real_process() {
        let tracker = Arc::new(ProcessTracker::new());

        // Spawn a long-running process.
        let child = Command::new("sleep")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn sleep");

        let pid = child.id();
        tracker.register(pid);

        // Cancel all tracked processes.
        tracker.cancel_all(&tokio::runtime::Handle::current());

        // Wait for the process to exit (SIGINT should kill it quickly).
        let output = tokio::task::spawn_blocking(move || child.wait_with_output())
            .await
            .expect("join failed")
            .expect("wait failed");

        tracker.unregister(pid);

        // Process should have been killed by signal (not exit code 0).
        assert!(!output.status.success());
        assert!(tracker.active_pids.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tracker_sigkill_fires_for_sigint_ignoring_process() {
        let tracker = Arc::new(ProcessTracker::new());

        // Spawn a process that traps SIGINT and ignores it.
        let child = Command::new("sh")
            .args(["-c", "trap '' INT; sleep 60"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn sh");

        let pid = child.id();
        tracker.register(pid);

        // Cancel - SIGINT is ignored, but SIGKILL fires after 2s grace period.
        tracker.cancel_all(&tokio::runtime::Handle::current());

        // Wait for the process to exit. Should be killed by SIGKILL within ~3s.
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::task::spawn_blocking(move || child.wait_with_output()),
        )
        .await
        .expect("process was not killed within 5s")
        .expect("join failed")
        .expect("wait failed");

        tracker.unregister(pid);

        assert!(!output.status.success());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tracker_cancel_all_multiple_processes() {
        let tracker = Arc::new(ProcessTracker::new());

        let child1 = Command::new("sleep")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn sleep 1");

        let child2 = Command::new("sleep")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn sleep 2");

        let pid1 = child1.id();
        let pid2 = child2.id();
        tracker.register(pid1);
        tracker.register(pid2);

        assert_eq!(tracker.active_pids.lock().unwrap().len(), 2);

        tracker.cancel_all(&tokio::runtime::Handle::current());

        let out1 = tokio::task::spawn_blocking(move || child1.wait_with_output())
            .await
            .expect("join 1 failed")
            .expect("wait 1 failed");

        let out2 = tokio::task::spawn_blocking(move || child2.wait_with_output())
            .await
            .expect("join 2 failed")
            .expect("wait 2 failed");

        tracker.unregister(pid1);
        tracker.unregister(pid2);

        assert!(!out1.status.success());
        assert!(!out2.status.success());
        assert!(tracker.active_pids.lock().unwrap().is_empty());
    }
}
