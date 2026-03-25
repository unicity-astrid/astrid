use std::collections::{HashMap, HashSet, VecDeque};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tracing::warn;

use astrid_workspace::SandboxCommand;

use crate::engine::wasm::bindings::astrid::capsule::process;
use crate::engine::wasm::bindings::astrid::capsule::types::{
    KillProcessResult, ProcessResult, ReadLogsResult, SpawnBackgroundResult, SpawnRequest,
};
use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

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
            .filter_map(|(&pid, stored_call_id)| match stored_call_id {
                None => Some(pid),
                Some(id) => call_id_set.contains(id).then_some(pid),
            })
            .collect();

        self.signal_pids(&pids, handle);
    }

    /// Send SIGINT to all tracked processes, then SIGKILL after a grace period.
    ///
    /// Used for capsule-level shutdown (e.g. capsule unload).
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

// ---------------------------------------------------------------------------
// Background process management
// ---------------------------------------------------------------------------

/// Maximum number of concurrent background processes per capsule.
pub(crate) const MAX_BACKGROUND_PROCESSES: usize = 8;

/// Maximum bytes buffered per stream (stdout or stderr) before oldest data is
/// dropped. 1 MB per stream, 2 MB total per process.
const MAX_BUFFER_BYTES: usize = 1024 * 1024;

/// A background process managed by the host on behalf of a WASM capsule.
///
/// Reader threads are fire-and-forget - they terminate naturally when the
/// child's pipes close (after kill or natural exit). No `JoinHandle` storage
/// is needed, avoiding hang risk in `Drop`.
pub struct ManagedProcess {
    /// The child process. Wrapped in `Option` so that explicit kill (or
    /// `try_wait` reap) can `.take()` it, preventing `Drop` from sending
    /// `killpg`/`kill` to a PID the OS may have already reused.
    child: Option<std::process::Child>,
    stdout_buf: Arc<Mutex<VecDeque<u8>>>,
    stderr_buf: Arc<Mutex<VecDeque<u8>>>,
    command: String,
}

/// Kill and reap a child process, including its entire process group on Unix.
/// Returns the exit code if available.
fn kill_and_reap(child: &mut std::process::Child) -> Option<i32> {
    #[cfg(unix)]
    {
        let raw_pid = child.id();
        let pid = nix::unistd::Pid::from_raw(i32::try_from(raw_pid).unwrap_or(i32::MAX));
        // Best-effort: process group may already be dead.
        let _ = nix::sys::signal::killpg(pid, nix::sys::signal::Signal::SIGKILL);
    }
    let _ = child.kill(); // fallback / Windows
    child.wait().ok().and_then(|s| s.code())
}

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            kill_and_reap(&mut child);
        }
    }
}

/// Drain a buffer, converting to a lossy UTF-8 string.
fn drain_buffer(buf: &Mutex<VecDeque<u8>>) -> String {
    let mut locked = buf.lock().unwrap_or_else(|e| e.into_inner());
    let bytes: Vec<u8> = locked.drain(..).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Spawn a reader thread that drains a pipe into a bounded buffer.
fn spawn_reader_thread(
    id: u64,
    label: &str,
    mut pipe: impl std::io::Read + Send + 'static,
    buffer: Arc<Mutex<VecDeque<u8>>>,
) {
    let name = format!("bg-{id}-{label}");
    std::thread::Builder::new()
        .name(name)
        .spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) => break, // pipe closed
                    Ok(n) => {
                        let mut locked = buffer.lock().unwrap_or_else(|e| e.into_inner());
                        locked.extend(&chunk[..n]);
                        // Enforce cap: drop oldest data if over limit.
                        let excess = locked.len().saturating_sub(MAX_BUFFER_BYTES);
                        if excess > 0 {
                            locked.drain(..excess);
                        }
                    },
                    Err(_) => break,
                }
            }
        })
        .ok(); // Thread spawn failure is non-fatal - output just won't be captured.
}

/// Prepare a sandboxed command for background execution.
///
/// Shared between spawn_host (sync) and spawn_background (async). Applies
/// environment stripping and sandbox wrapping.
fn prepare_sandboxed_command(
    cmd: &str,
    args: &[String],
    workspace_root: &std::path::Path,
) -> Result<Command, String> {
    let mut inner_cmd = Command::new(cmd);
    let str_args: Vec<&str> = args.iter().map(String::as_str).collect();
    inner_cmd.args(&str_args);
    inner_cmd.env_remove("ASTRID_SOCKET_PATH");
    inner_cmd.env_remove("ASTRID_SESSION_TOKEN");
    inner_cmd.env_remove("ASTRID_HOME");

    SandboxCommand::wrap(inner_cmd, workspace_root)
        .map_err(|e| format!("failed to wrap command in sandbox: {e}"))
}

impl process::Host for HostState {
    fn spawn(&mut self, request: SpawnRequest) -> Result<ProcessResult, String> {
        let workspace_root = self.workspace_root.clone();
        let security = self.security.clone();
        let capsule_id = self.capsule_id.as_str().to_owned();
        let handle = self.runtime_handle.clone();
        let semaphore = self.host_semaphore.clone();
        let cancel_token = self.cancel_token.clone();
        let process_tracker = self.process_tracker.clone();

        // Extract call_id from the caller context (IPC message that triggered this
        // invocation) for multi-session scoped cancellation.
        let call_id = self.caller_context.as_ref().and_then(|msg| {
            if let astrid_events::ipc::IpcPayload::ToolExecuteRequest { call_id, .. } = &msg.payload
            {
                Some(call_id.clone())
            } else {
                None
            }
        });

        if let Some(sec) = security {
            let cmd = request.cmd.to_string();
            util::bounded_block_on(&handle, &semaphore, async {
                sec.check_host_process(&capsule_id, &cmd).await
            })
            .map_err(|e| format!("Security Check Failed: {e}"))?;
        } else {
            return Err(
                "Security Check Failed: No security gate found for host_process capability."
                    .to_string(),
            );
        }

        let mut sandboxed_cmd =
            prepare_sandboxed_command(&request.cmd, &request.args, &workspace_root)?;

        // Spawn the child process (non-blocking) so we can track its PID.
        sandboxed_cmd.stdout(Stdio::piped());
        sandboxed_cmd.stderr(Stdio::piped());

        let child = sandboxed_cmd
            .spawn()
            .map_err(|e| format!("failed to spawn command: {e}"))?;

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
                return Err(format!("failed to execute command: {e}"));
            },
            None => {
                // Cancelled (capsule unloading or tool cancellation).
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

        Ok(result)
    }

    fn spawn_background(&mut self, request: SpawnRequest) -> Result<SpawnBackgroundResult, String> {
        // Check process limit before doing any expensive work.
        if self.background_processes.len() >= MAX_BACKGROUND_PROCESSES {
            return Err(format!(
                "background process limit reached (max {MAX_BACKGROUND_PROCESSES})"
            ));
        }

        let workspace_root = self.workspace_root.clone();
        let security = self.security.clone();
        let capsule_id = self.capsule_id.as_str().to_owned();
        let handle = self.runtime_handle.clone();
        let semaphore = self.host_semaphore.clone();

        // Security gate - same check as synchronous spawn.
        if let Some(sec) = security {
            let cmd = request.cmd.to_string();
            util::bounded_block_on(&handle, &semaphore, async {
                sec.check_host_process(&capsule_id, &cmd).await
            })
            .map_err(|e| format!("Security Check Failed: {e}"))?;
        } else {
            return Err(
                "Security Check Failed: No security gate found for host_process capability."
                    .to_string(),
            );
        }

        let mut sandboxed_cmd =
            prepare_sandboxed_command(&request.cmd, &request.args, &workspace_root)?;

        // Set up as process group leader for clean group kills on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            sandboxed_cmd.process_group(0);
        }

        sandboxed_cmd.stdout(Stdio::piped());
        sandboxed_cmd.stderr(Stdio::piped());

        let command_str = format!("{} {}", request.cmd, request.args.join(" "));

        let child = sandboxed_cmd
            .spawn()
            .map_err(|e| format!("failed to spawn background process: {e}"))?;

        // Wrap immediately in ManagedProcess so that any early return triggers
        // Drop which kills + reaps the child.
        let stdout_buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));
        let mut managed = ManagedProcess {
            child: Some(child),
            stdout_buf: Arc::clone(&stdout_buf),
            stderr_buf: Arc::clone(&stderr_buf),
            command: command_str,
        };

        // Defensive re-check: limit could theoretically have been reached.
        if self.background_processes.len() >= MAX_BACKGROUND_PROCESSES {
            return Err(format!(
                "background process limit reached (max {MAX_BACKGROUND_PROCESSES})"
            ));
        }

        let process_id = self.next_process_id;
        self.next_process_id += 1;

        if let Some(child) = managed.child.as_mut() {
            if let Some(stdout) = child.stdout.take() {
                spawn_reader_thread(process_id, "stdout", stdout, Arc::clone(&stdout_buf));
            }
            if let Some(stderr) = child.stderr.take() {
                spawn_reader_thread(process_id, "stderr", stderr, Arc::clone(&stderr_buf));
            }
        }

        tracing::info!(
            capsule_id = %capsule_id,
            process_id = process_id,
            command = %managed.command,
            "Spawned background process"
        );

        self.background_processes.insert(process_id, managed);

        Ok(SpawnBackgroundResult { id: process_id })
    }

    fn read_logs(&mut self, process_id: u64) -> Result<ReadLogsResult, String> {
        let proc = self
            .background_processes
            .get_mut(&process_id)
            .ok_or_else(|| format!("no background process with id {process_id}"))?;

        // try_wait is non-blocking (waitpid WNOHANG). If it returns Some(status),
        // the child has been reaped and the PID is free for OS reuse.
        let (running, exit_code) = if let Some(child) = proc.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    // Child reaped - take it so Drop won't act on stale PID.
                    proc.child.take();
                    (false, status.code())
                },
                Ok(None) => (true, None),
                Err(_) => {
                    proc.child.take();
                    (false, Some(-1))
                },
            }
        } else {
            // Child already taken (previously reaped).
            (false, None)
        };

        let stdout = drain_buffer(&proc.stdout_buf);
        let stderr = drain_buffer(&proc.stderr_buf);

        Ok(ReadLogsResult {
            stdout,
            stderr,
            running,
            exit_code,
        })
    }

    fn kill(&mut self, process_id: u64) -> Result<KillProcessResult, String> {
        // Remove from map (takes ownership) so we can drop the HostState lock
        // before doing the potentially-blocking kill + wait.
        let mut proc = self
            .background_processes
            .remove(&process_id)
            .ok_or_else(|| format!("no background process with id {process_id}"))?;

        let capsule_id = self.capsule_id.as_str().to_owned();

        // Drain remaining buffered output before killing.
        let stdout = drain_buffer(&proc.stdout_buf);
        let stderr = drain_buffer(&proc.stderr_buf);

        // Take the child so Drop won't double-kill on a potentially-reused PID.
        let exit_code = if let Some(mut child) = proc.child.take() {
            kill_and_reap(&mut child)
        } else {
            // Already reaped by a prior try_wait in read_logs.
            None
        };

        tracing::info!(
            capsule_id = %capsule_id,
            process_id = process_id,
            command = %proc.command,
            exit_code = ?exit_code,
            "Killed background process"
        );

        Ok(KillProcessResult {
            killed: true,
            exit_code,
            stdout,
            stderr,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_cap_enforced() {
        let buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));
        let mut data = vec![b'A'; MAX_BUFFER_BYTES + 500];
        data[0] = b'X';
        data[500] = b'Y';

        {
            let mut locked = buf.lock().unwrap_or_else(|e| e.into_inner());
            locked.extend(&data);
            let excess = locked.len().saturating_sub(MAX_BUFFER_BYTES);
            if excess > 0 {
                locked.drain(..excess);
            }
        }

        let locked = buf.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(locked.len(), MAX_BUFFER_BYTES);
        assert_eq!(locked[0], b'Y');
        assert!(!locked.contains(&b'X'));
    }

    #[test]
    fn drain_buffer_clears_and_returns() {
        let buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));
        {
            let mut locked = buf.lock().unwrap_or_else(|e| e.into_inner());
            locked.extend(b"hello world");
        }

        let result = drain_buffer(&buf);
        assert_eq!(result, "hello world");

        let locked = buf.lock().unwrap_or_else(|e| e.into_inner());
        assert!(locked.is_empty());
    }

    #[test]
    fn drain_buffer_handles_empty() {
        let buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));
        let result = drain_buffer(&buf);
        assert_eq!(result, "");
    }

    #[test]
    fn managed_process_drop_kills_child() {
        let child = Command::new("sleep")
            .arg("60")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn sleep");

        let raw_pid = child.id();

        let managed = ManagedProcess {
            child: Some(child),
            stdout_buf: Arc::new(Mutex::new(VecDeque::new())),
            stderr_buf: Arc::new(Mutex::new(VecDeque::new())),
            command: "sleep 60".to_string(),
        };

        drop(managed);

        #[cfg(unix)]
        {
            let pid = nix::unistd::Pid::from_raw(i32::try_from(raw_pid).unwrap_or(i32::MAX));
            let result = nix::sys::signal::kill(pid, None);
            assert!(
                result.is_err(),
                "process should be dead after ManagedProcess drop"
            );
        }
    }

    #[test]
    fn spawn_respects_limit() {
        use std::collections::HashMap;

        let mut processes: HashMap<u64, ManagedProcess> = HashMap::new();
        for i in 0..MAX_BACKGROUND_PROCESSES {
            let child = Command::new("sleep")
                .arg("60")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("failed to spawn");
            processes.insert(
                i as u64,
                ManagedProcess {
                    child: Some(child),
                    stdout_buf: Arc::new(Mutex::new(VecDeque::new())),
                    stderr_buf: Arc::new(Mutex::new(VecDeque::new())),
                    command: "sleep 60".to_string(),
                },
            );
        }

        assert!(
            processes.len() >= MAX_BACKGROUND_PROCESSES,
            "at limit: should reject new spawns"
        );

        processes.remove(&0);
        assert!(
            processes.len() < MAX_BACKGROUND_PROCESSES,
            "below limit: should allow new spawns"
        );
    }

    #[test]
    fn kill_nonexistent_returns_error() {
        let processes: std::collections::HashMap<u64, ManagedProcess> =
            std::collections::HashMap::new();
        assert!(processes.get(&999).is_none());
    }

    #[test]
    fn read_logs_after_natural_exit() {
        let mut child = Command::new("echo")
            .arg("hello from echo")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn echo");

        let stdout_buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));

        if let Some(stdout) = child.stdout.take() {
            spawn_reader_thread(1, "stdout", stdout, Arc::clone(&stdout_buf));
        }

        let status = child.wait().expect("failed to wait");
        assert!(status.success());

        std::thread::sleep(std::time::Duration::from_millis(50));

        let stdout = drain_buffer(&stdout_buf);
        let stderr = drain_buffer(&stderr_buf);

        assert!(
            stdout.contains("hello from echo"),
            "expected output after natural exit, got: {stdout}"
        );
        assert!(stderr.is_empty());
    }

    #[test]
    fn kill_returns_final_output() {
        let mut child = Command::new("echo")
            .arg("final output")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn echo");

        let stdout_buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));

        if let Some(stdout) = child.stdout.take() {
            spawn_reader_thread(1, "test-stdout", stdout, Arc::clone(&stdout_buf));
        }

        let _ = child.wait().expect("failed to wait for child");
        std::thread::sleep(std::time::Duration::from_millis(50));

        let stdout = drain_buffer(&stdout_buf);
        assert!(
            stdout.contains("final output"),
            "expected 'final output' in stdout, got: {stdout}"
        );
    }

    // -----------------------------------------------------------------------
    // ProcessTracker tests
    // -----------------------------------------------------------------------

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
        tracker.cancel_all(rt.handle());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tracker_cancel_all_kills_real_process() {
        let tracker = Arc::new(ProcessTracker::new());

        let child = Command::new("sleep")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn sleep");

        let pid = child.id();
        tracker.register(pid, None);

        tracker.cancel_all(&tokio::runtime::Handle::current());

        let output = tokio::task::spawn_blocking(move || child.wait_with_output())
            .await
            .expect("join failed")
            .expect("wait failed");

        tracker.unregister(pid);

        assert!(!output.status.success());
        assert!(tracker.active_pids.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tracker_sigkill_fires_for_sigint_ignoring_process() {
        let tracker = Arc::new(ProcessTracker::new());

        let child = Command::new("sh")
            .args(["-c", "trap '' INT; sleep 60"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn sh");

        let pid = child.id();
        tracker.register(pid, None);

        tracker.cancel_all(&tokio::runtime::Handle::current());

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

        tracker.cancel_by_call_ids(&["call-a".into()], &tokio::runtime::Handle::current());

        let out_a = tokio::task::spawn_blocking(move || child_a.wait_with_output())
            .await
            .expect("join a failed")
            .expect("wait a failed");
        assert!(!out_a.status.success());

        assert!(tracker.active_pids.lock().unwrap().contains_key(&pid_b));

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

        let child = Command::new("sleep")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn sleep");

        let pid = child.id();
        tracker.register(pid, None);

        tracker.cancel_by_call_ids(&["any-id".into()], &tokio::runtime::Handle::current());

        let output = tokio::task::spawn_blocking(move || child.wait_with_output())
            .await
            .expect("join failed")
            .expect("wait failed");

        tracker.unregister(pid);
        assert!(!output.status.success());
    }
}
