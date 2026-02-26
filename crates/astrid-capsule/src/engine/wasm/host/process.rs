use extism::{CurrentPlugin, Error, UserData, Val};
use serde::{Deserialize, Serialize};

use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
use astrid_workspace::SandboxCommand;
use std::process::Command;

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

#[allow(clippy::needless_pass_by_value)]
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
    drop(state);

    if let Some(sec) = security {
        let cmd = req.cmd.to_string();
        tokio::task::block_in_place(|| {
            handle.block_on(async { sec.check_host_process(&capsule_id, &cmd).await })
        })
        .map_err(|e| Error::msg(format!("Security Check Failed: {e}")))?;
    } else {
        return Err(Error::msg(
            "Security Check Failed: No security gate found for host_process capability.",
        ));
    }

    let mut inner_cmd = Command::new(req.cmd);
    inner_cmd.args(&req.args);

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
