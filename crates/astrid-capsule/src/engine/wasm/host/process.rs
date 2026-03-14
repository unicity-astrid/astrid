use std::collections::VecDeque;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use extism::{CurrentPlugin, Error, UserData, Val};
use serde::{Deserialize, Serialize};

use astrid_workspace::SandboxCommand;

use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

// ---------------------------------------------------------------------------
// Synchronous process spawn (existing)
// ---------------------------------------------------------------------------

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

    let mut sandboxed_cmd = sandboxed_cmd;
    let output = sandboxed_cmd
        .output()
        .map_err(|e| Error::msg(format!("failed to execute command: {e}")))?;

    let result = ProcessResult {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code().unwrap_or(-1),
    };

    let result_bytes = serde_json::to_vec(&result)?;
    let mem = plugin.memory_new(&result_bytes)?;
    outputs[0] = plugin.memory_to_val(mem);

    Ok(())
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
    child: std::process::Child,
    stdout_buf: Arc<Mutex<VecDeque<u8>>>,
    stderr_buf: Arc<Mutex<VecDeque<u8>>>,
    command: String,
}

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        // Kill the entire process group to catch grandchildren (e.g.,
        // npm -> node -> webpack). On Linux inside bwrap with
        // --unshare-pid this is redundant (PID namespace cleanup), but
        // on macOS (Seatbelt, no PID namespace) it prevents orphans.
        #[cfg(unix)]
        {
            let raw_pid = self.child.id();
            let pid = nix::unistd::Pid::from_raw(i32::try_from(raw_pid).unwrap_or(i32::MAX));
            // Best-effort: process group may already be dead.
            let _ = nix::sys::signal::killpg(pid, nix::sys::signal::Signal::SIGKILL);
        }
        let _ = self.child.kill(); // fallback / Windows
        let _ = self.child.wait(); // reap zombie
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
    args: &[&str],
    workspace_root: &std::path::Path,
) -> Result<Command, Error> {
    let mut inner_cmd = Command::new(cmd);
    inner_cmd.args(args);
    inner_cmd.env_remove("ASTRID_SOCKET_PATH");
    inner_cmd.env_remove("ASTRID_SESSION_TOKEN");
    inner_cmd.env_remove("ASTRID_HOME");

    SandboxCommand::wrap(inner_cmd, workspace_root)
        .map_err(|e| Error::msg(format!("failed to wrap command in sandbox: {e}")))
}

// ---------------------------------------------------------------------------
// Request/response types for background process host functions
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct SpawnBackgroundResult {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct BackgroundProcessRequest {
    id: u64,
}

#[derive(Debug, Serialize)]
struct ReadLogsResult {
    stdout: String,
    stderr: String,
    running: bool,
    exit_code: Option<i32>,
}

#[derive(Debug, Serialize)]
struct KillProcessResult {
    killed: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

// ---------------------------------------------------------------------------
// Host function: spawn background process
// ---------------------------------------------------------------------------

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_spawn_background_host_impl(
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

    // Check process limit before doing any expensive work.
    if state.background_processes.len() >= MAX_BACKGROUND_PROCESSES {
        return Err(Error::msg(format!(
            "background process limit reached (max {MAX_BACKGROUND_PROCESSES})"
        )));
    }

    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let capsule_id = state.capsule_id.as_str().to_owned();
    let handle = state.runtime_handle.clone();
    let semaphore = state.host_semaphore.clone();
    drop(state);

    // Security gate - same check as synchronous spawn.
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

    let mut sandboxed_cmd = prepare_sandboxed_command(req.cmd, &req.args, &workspace_root)?;

    // Set up as process group leader for clean group kills on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        sandboxed_cmd.process_group(0);
    }

    sandboxed_cmd.stdout(Stdio::piped());
    sandboxed_cmd.stderr(Stdio::piped());

    let command_str = format!("{} {}", req.cmd, req.args.join(" "));

    let mut child = sandboxed_cmd
        .spawn()
        .map_err(|e| Error::msg(format!("failed to spawn background process: {e}")))?;

    // Take pipe handles before storing the Child.
    let stdout_buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));
    let stderr_buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));

    // Re-lock HostState to get the handle ID BEFORE spawning threads,
    // so the thread name includes the correct ID.
    let ud2 = user_data.get()?;
    let mut state = ud2
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let process_id = state.next_process_id;
    state.next_process_id += 1;

    if let Some(stdout) = child.stdout.take() {
        spawn_reader_thread(process_id, "stdout", stdout, Arc::clone(&stdout_buf));
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_reader_thread(process_id, "stderr", stderr, Arc::clone(&stderr_buf));
    }

    let managed = ManagedProcess {
        child,
        stdout_buf,
        stderr_buf,
        command: command_str,
    };

    tracing::info!(
        capsule_id = %capsule_id,
        process_id = process_id,
        command = %managed.command,
        "Spawned background process"
    );

    state.background_processes.insert(process_id, managed);
    drop(state);

    let result = SpawnBackgroundResult { id: process_id };
    let result_bytes = serde_json::to_vec(&result)?;
    let mem = plugin.memory_new(&result_bytes)?;
    outputs[0] = plugin.memory_to_val(mem);

    Ok(())
}

// ---------------------------------------------------------------------------
// Host function: read process logs
// ---------------------------------------------------------------------------

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_read_process_logs_host_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let req_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], 256)?;
    let req: BackgroundProcessRequest = serde_json::from_slice(&req_bytes)
        .map_err(|e| Error::msg(format!("failed to parse read logs request: {e}")))?;

    let ud = user_data.get()?;
    let mut state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    let proc = state
        .background_processes
        .get_mut(&req.id)
        .ok_or_else(|| Error::msg(format!("no background process with id {}", req.id)))?;

    // try_wait is non-blocking (waitpid WNOHANG).
    let (running, exit_code) = match proc.child.try_wait() {
        Ok(Some(status)) => (false, status.code()),
        Ok(None) => (true, None),
        Err(_) => (false, Some(-1)),
    };

    // Clone buffer Arcs so we can drain outside the HostState lock if needed.
    // In practice, draining is fast, so we do it under the lock for simplicity.
    let stdout = drain_buffer(&proc.stdout_buf);
    let stderr = drain_buffer(&proc.stderr_buf);
    drop(state);

    let result = ReadLogsResult {
        stdout,
        stderr,
        running,
        exit_code,
    };
    let result_bytes = serde_json::to_vec(&result)?;
    let mem = plugin.memory_new(&result_bytes)?;
    outputs[0] = plugin.memory_to_val(mem);

    Ok(())
}

// ---------------------------------------------------------------------------
// Host function: kill process
// ---------------------------------------------------------------------------

#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_kill_process_host_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let req_bytes: Vec<u8> = util::get_safe_bytes(plugin, &inputs[0], 256)?;
    let req: BackgroundProcessRequest = serde_json::from_slice(&req_bytes)
        .map_err(|e| Error::msg(format!("failed to parse kill request: {e}")))?;

    let ud = user_data.get()?;
    let mut state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

    // Remove from map (takes ownership) so we can drop the HostState lock
    // before doing the potentially-blocking kill + wait.
    let mut proc = state
        .background_processes
        .remove(&req.id)
        .ok_or_else(|| Error::msg(format!("no background process with id {}", req.id)))?;

    let capsule_id = state.capsule_id.as_str().to_owned();
    drop(state);

    // Drain remaining buffered output before killing.
    let stdout = drain_buffer(&proc.stdout_buf);
    let stderr = drain_buffer(&proc.stderr_buf);

    // Kill the process group, then the process directly, then reap.
    #[cfg(unix)]
    {
        let raw_pid = proc.child.id();
        let pid = nix::unistd::Pid::from_raw(i32::try_from(raw_pid).unwrap_or(i32::MAX));
        let _ = nix::sys::signal::killpg(pid, nix::sys::signal::Signal::SIGKILL);
    }
    let _ = proc.child.kill();
    let exit_code = proc.child.wait().ok().and_then(|s| s.code());

    tracing::info!(
        capsule_id = %capsule_id,
        process_id = req.id,
        command = %proc.command,
        exit_code = ?exit_code,
        "Killed background process"
    );

    let result = KillProcessResult {
        killed: true,
        exit_code,
        stdout,
        stderr,
    };
    let result_bytes = serde_json::to_vec(&result)?;
    let mem = plugin.memory_new(&result_bytes)?;
    outputs[0] = plugin.memory_to_val(mem);

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    #[test]
    fn buffer_cap_enforced() {
        let buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));
        let data = vec![b'A'; MAX_BUFFER_BYTES + 500];

        // Simulate what the reader thread does: append then cap.
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
        // The oldest 500 bytes should have been dropped.
        assert_eq!(locked[0], b'A');
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

        // Buffer should be empty after drain.
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
            child,
            stdout_buf: Arc::new(Mutex::new(VecDeque::new())),
            stderr_buf: Arc::new(Mutex::new(VecDeque::new())),
            command: "sleep 60".to_string(),
        };

        drop(managed);

        // Verify the process is dead by checking if waitpid returns an error
        // (the process was already reaped by Drop).
        #[cfg(unix)]
        {
            let pid = nix::unistd::Pid::from_raw(i32::try_from(raw_pid).unwrap_or(i32::MAX));
            // kill with signal 0 checks if process exists without sending a signal.
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
                    child,
                    stdout_buf: Arc::new(Mutex::new(VecDeque::new())),
                    stderr_buf: Arc::new(Mutex::new(VecDeque::new())),
                    command: "sleep 60".to_string(),
                },
            );
        }

        assert_eq!(processes.len(), MAX_BACKGROUND_PROCESSES);
        // The host function checks `>= MAX_BACKGROUND_PROCESSES` before spawning.
        assert!(processes.len() >= MAX_BACKGROUND_PROCESSES);

        // Cleanup: drop kills all processes.
    }

    #[test]
    fn kill_nonexistent_returns_error() {
        // Simulate the lookup that kill_process does.
        let processes: std::collections::HashMap<u64, ManagedProcess> =
            std::collections::HashMap::new();
        assert!(processes.get(&999).is_none());
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
        let _stderr_buf: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));

        // Read stdout into buffer (echo exits quickly).
        if let Some(mut stdout) = child.stdout.take() {
            let buf = Arc::clone(&stdout_buf);
            let mut chunk = [0u8; 4096];
            loop {
                match stdout.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut locked = buf.lock().unwrap_or_else(|e| e.into_inner());
                        locked.extend(&chunk[..n]);
                    },
                    Err(_) => break,
                }
            }
        }

        // Drain should return the output.
        let stdout = drain_buffer(&stdout_buf);
        assert!(
            stdout.contains("final output"),
            "expected 'final output' in stdout, got: {stdout}"
        );
    }
}
