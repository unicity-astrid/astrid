use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

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

/// Grace period between SIGINT and SIGKILL when cancelling processes.
const SIGKILL_GRACE_PERIOD: Duration = Duration::from_secs(2);

/// Tracks active child process PIDs for cancellation, with optional call_id
/// association for multi-session scoping.
///
/// Each PID is mapped to an optional `call_id` (the tool call identifier from
/// the React loop's `ToolExecuteRequest`). When a cancel event arrives with
/// specific `call_ids`, only processes matching those IDs are killed. Processes
/// with no call_id (None) are always included in targeted cancellation as a
/// conservative fallback for code paths that haven't threaded call_id through.
///
/// Shared between the spawn host function (registers/unregisters PIDs) and the
/// cancel listener background task (sends SIGINT/SIGKILL on cancellation).
#[derive(Debug, Default)]
pub struct ProcessTracker {
    /// Maps PID -> optional call_id.
    active_pids: std::sync::Arc<Mutex<HashMap<u32, Option<String>>>>,
}

impl ProcessTracker {
    /// Create a new, empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a child process PID with an optional call_id for scoped
    /// cancellation.
    pub fn register(&self, pid: u32, call_id: Option<String>) {
        if pid == 0 {
            return; // Guard: PID 0 means "no process" on some platforms.
        }
        self.active_pids
            .lock()
            .expect("process tracker lock poisoned")
            .insert(pid, call_id);
    }

    /// Unregister a child process PID (process has exited).
    pub fn unregister(&self, pid: u32) {
        self.active_pids
            .lock()
            .expect("process tracker lock poisoned")
            .remove(&pid);
    }

    /// Cancel processes matching the given call_ids.
    ///
    /// Kills processes whose call_id matches one of the provided IDs, plus
    /// any processes with no call_id (conservative fallback for code paths
    /// that haven't threaded call_id through yet). Processes with a
    /// *different* call_id are left untouched.
    ///
    /// Sends SIGINT first, then SIGKILL after a 2-second grace period. The
    /// SIGKILL task re-checks `active_pids` before signaling to avoid
    /// hitting reused PIDs.
    pub fn cancel_by_call_ids(&self, call_ids: &[String], handle: &tokio::runtime::Handle) {
        if call_ids.is_empty() {
            return;
        }
        let call_id_set: HashSet<&String> = call_ids.iter().collect();

        let pids: Vec<u32> = self
            .active_pids
            .lock()
            .expect("process tracker lock poisoned")
            .iter()
            .filter_map(|(&pid, stored_call_id)| {
                match stored_call_id {
                    // No call_id stored: conservative fallback, always include.
                    None => Some(pid),
                    // Has call_id: only include if it matches one of the target IDs.
                    Some(id) => call_id_set.contains(id).then_some(pid),
                }
            })
            .collect();

        self.signal_pids(&pids, handle);
    }

    /// Send SIGINT to all tracked processes, then SIGKILL after a grace period.
    ///
    /// Used for capsule-level shutdown (e.g. capsule unload). For session-scoped
    /// cancellation, use [`cancel_by_call_ids`](Self::cancel_by_call_ids).
    pub fn cancel_all(&self, handle: &tokio::runtime::Handle) {
        let pids: Vec<u32> = self
            .active_pids
            .lock()
            .expect("process tracker lock poisoned")
            .keys()
            .copied()
            .collect();

        self.signal_pids(&pids, handle);
    }

    /// Send SIGINT to the given PIDs, then SIGKILL survivors after 2 seconds.
    ///
    /// On macOS, `sandbox-exec` replaces itself via `exec()`, so the tracked
    /// PID IS the real inner command. On Linux, `bwrap` forwards signals to
    /// the inner process. Known limitation: if a future sandbox wrapper forks
    /// without forwarding signals, the inner process may survive SIGINT.
    /// The SIGKILL task re-checks `active_pids` before signaling to avoid
    /// hitting reused PIDs.
    fn signal_pids(&self, pids: &[u32], handle: &tokio::runtime::Handle) {
        if pids.is_empty() {
            return;
        }

        // SIGINT all targeted processes.
        for &pid in pids {
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
        let target_pids: Vec<u32> = pids.to_vec();
        handle.spawn(async move {
            tokio::time::sleep(SIGKILL_GRACE_PERIOD).await;
            let still_active = tracker.lock().expect("process tracker lock poisoned");
            for pid in target_pids {
                // Only signal PIDs still in the tracker (not yet unregistered).
                if !still_active.contains_key(&pid) {
                    continue;
                }
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

    // Extract call_id from the caller context (IPC message that triggered this
    // invocation) for multi-session scoped cancellation. When the caller
    // context is a ToolExecuteRequest, the call_id identifies which specific
    // tool invocation this process belongs to.
    let call_id = state.caller_context.as_ref().and_then(|msg| {
        if let astrid_events::ipc::IpcPayload::ToolExecuteRequest { call_id, .. } = &msg.payload {
            Some(call_id.clone())
        } else {
            None
        }
    });
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
    process_tracker.register(pid, call_id);

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
        tracker.register(1234, None);
        tracker.register(5678, Some("call-a".into()));
        assert_eq!(tracker.active_pids.lock().unwrap().len(), 2);
        tracker.unregister(1234);
        assert_eq!(tracker.active_pids.lock().unwrap().len(), 1);
        assert!(tracker.active_pids.lock().unwrap().contains_key(&5678));
    }

    #[test]
    fn tracker_ignores_pid_zero() {
        let tracker = ProcessTracker::new();
        tracker.register(0, None);
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
        tracker.register(pid, None);

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
        tracker.register(pid, None);

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
        tracker.register(pid1, None);
        tracker.register(pid2, None);

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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tracker_cancel_by_call_ids_scoped() {
        let tracker = Arc::new(ProcessTracker::new());

        // Spawn two processes with different call_ids.
        let child_a = Command::new("sleep")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn sleep a");

        let child_b = Command::new("sleep")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn sleep b");

        let pid_a = child_a.id();
        let pid_b = child_b.id();
        tracker.register(pid_a, Some("call-a".into()));
        tracker.register(pid_b, Some("call-b".into()));

        // Cancel only call-a.
        tracker.cancel_by_call_ids(&["call-a".into()], &tokio::runtime::Handle::current());

        // child_a should be killed.
        let out_a = tokio::task::spawn_blocking(move || child_a.wait_with_output())
            .await
            .expect("join a failed")
            .expect("wait a failed");
        assert!(!out_a.status.success());

        // child_b should still be tracked (alive).
        assert!(tracker.active_pids.lock().unwrap().contains_key(&pid_b));

        // Clean up child_b.
        if let Some(raw) = i32::try_from(pid_b).ok() {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(raw),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        let _ = tokio::task::spawn_blocking(move || child_b.wait_with_output()).await;
        tracker.unregister(pid_a);
        tracker.unregister(pid_b);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tracker_cancel_by_call_ids_includes_none() {
        let tracker = Arc::new(ProcessTracker::new());

        // Process with no call_id (legacy/unthreaded path).
        let child = Command::new("sleep")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn sleep");

        let pid = child.id();
        tracker.register(pid, None);

        // cancel_by_call_ids should include None-call_id processes.
        tracker.cancel_by_call_ids(&["any-id".into()], &tokio::runtime::Handle::current());

        let output = tokio::task::spawn_blocking(move || child.wait_with_output())
            .await
            .expect("join failed")
            .expect("wait failed");

        tracker.unregister(pid);
        assert!(!output.status.success());
    }
}
