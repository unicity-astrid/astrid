//! Extism host function implementations matching the WIT `host` interface.
//!
//! Seven host functions are registered with every Extism plugin instance:
//!
//! | Function | Security Gate | Async Bridge |
//! |----------|--------------|--------------|
//! | `astrid_log` | No | No |
//! | `astrid_http_request` | Yes | Yes |
//! | `astrid_read_file` | Yes | Yes |
//! | `astrid_write_file` | Yes | Yes |
//! | `astrid_kv_get` | No | Yes |
//! | `astrid_kv_set` | No | Yes |
//! | `astrid_get_config` | No | No |
//!
//! All host functions use `UserData<HostState>` for shared state access.
//! Async operations are bridged via `Handle::block_on()` — this requires
//! the **multi-threaded** tokio runtime.

use std::path::Path;

use extism::{CurrentPlugin, Error, PTR, UserData, Val};

#[cfg(feature = "http")]
use astrid_core::plugin_abi::HttpResponse;
use astrid_core::plugin_abi::{KeyValuePair, LogLevel};

use super::host_state::HostState;

// ---------------------------------------------------------------------------
// astrid_log(level, message)
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_log_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let level: String = plugin.memory_get_val(&inputs[0])?;
    let message: String = plugin.memory_get_val(&inputs[1])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    drop(state);

    let parsed_level: LogLevel =
        serde_json::from_str(&format!("\"{level}\"")).unwrap_or(LogLevel::Info);

    match parsed_level {
        LogLevel::Trace => tracing::trace!(plugin = %plugin_id, "{message}"),
        LogLevel::Debug => tracing::debug!(plugin = %plugin_id, "{message}"),
        LogLevel::Info => tracing::info!(plugin = %plugin_id, "{message}"),
        LogLevel::Warn => tracing::warn!(plugin = %plugin_id, "{message}"),
        LogLevel::Error => tracing::error!(plugin = %plugin_id, "{message}"),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_get_config(key) -> value_json
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_get_config_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let key: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let value = state.config.get(&key).cloned();
    drop(state);

    let result = match value {
        Some(v) => serde_json::to_string(&v).unwrap_or_default(),
        None => String::new(),
    };

    let mem = plugin.memory_new(&result)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_kv_get(key) -> value
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_kv_get_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let key: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let kv = state.kv.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let result = handle.block_on(async { kv.get(&key).await });

    let value = match result {
        Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        Ok(None) => String::new(),
        Err(e) => return Err(Error::msg(format!("kv_get failed: {e}"))),
    };

    let mem = plugin.memory_new(&value)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_kv_set(key, value)
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_kv_set_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let key: String = plugin.memory_get_val(&inputs[0])?;
    let value: String = plugin.memory_get_val(&inputs[1])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let kv = state.kv.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    let result = handle.block_on(async { kv.set(&key, value.into_bytes()).await });

    match result {
        Ok(()) => Ok(()),
        Err(e) => Err(Error::msg(format!("kv_set failed: {e}"))),
    }
}

// ---------------------------------------------------------------------------
// astrid_read_file(path) -> content
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_read_file_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = plugin.memory_get_val(&inputs[0])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    // Resolve and confine path to workspace
    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    // Security check
    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_read(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied file read: {reason}")));
        }
    }

    // Read file
    let content = std::fs::read_to_string(&resolved)
        .map_err(|e| Error::msg(format!("read_file failed ({resolved_str}): {e}")))?;

    let mem = plugin.memory_new(&content)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_write_file(path, content)
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_write_file_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let path: String = plugin.memory_get_val(&inputs[0])?;
    let content: String = plugin.memory_get_val(&inputs[1])?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let workspace_root = state.workspace_root.clone();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    // Resolve and confine path to workspace
    let resolved = resolve_within_workspace(&workspace_root, &path)?;
    let resolved_str = resolved.to_string_lossy().to_string();

    // Security check
    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let rstr = resolved_str.clone();
        let check = handle.block_on(async move { gate.check_file_write(&pid, &rstr).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!("security denied file write: {reason}")));
        }
    }

    // Write file
    std::fs::write(&resolved, content.as_bytes())
        .map_err(|e| Error::msg(format!("write_file failed ({resolved_str}): {e}")))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// astrid_http_request(request_json) -> response_json
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astrid_http_request_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct HttpRequest {
        method: String,
        url: String,
        #[serde(default)]
        headers: Vec<KeyValuePair>,
        #[serde(default)]
        body: Option<String>,
    }

    let request_json: String = plugin.memory_get_val(&inputs[0])?;

    let req: HttpRequest = serde_json::from_str(&request_json)
        .map_err(|e| Error::msg(format!("invalid HTTP request JSON: {e}")))?;

    let ud = user_data.get()?;
    let state = ud
        .lock()
        .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
    let plugin_id = state.plugin_id.as_str().to_owned();
    let security = state.security.clone();
    let handle = state.runtime_handle.clone();
    drop(state);

    // Security check
    if let Some(gate) = &security {
        let gate = gate.clone();
        let pid = plugin_id.clone();
        let method = req.method.clone();
        let url = req.url.clone();
        let check =
            handle.block_on(async move { gate.check_http_request(&pid, &method, &url).await });
        if let Err(reason) = check {
            return Err(Error::msg(format!(
                "security denied HTTP request: {reason}"
            )));
        }
    }

    // Perform the HTTP request (feature-gated)
    #[cfg(feature = "http")]
    {
        let response = handle.block_on(async {
            perform_http_request(&req.method, &req.url, &req.headers, req.body.as_deref()).await
        })?;
        let response_json = serde_json::to_string(&response)
            .map_err(|e| Error::msg(format!("failed to serialize HTTP response: {e}")))?;
        let mem = plugin.memory_new(&response_json)?;
        outputs[0] = plugin.memory_to_val(mem);
        Ok(())
    }

    #[cfg(not(feature = "http"))]
    {
        let _ = outputs;
        Err(Error::msg(
            "HTTP support not enabled — enable the 'http' feature on astrid-plugins",
        ))
    }
}

// ---------------------------------------------------------------------------
// HTTP implementation (behind feature flag)
// ---------------------------------------------------------------------------

#[cfg(feature = "http")]
async fn perform_http_request(
    method: &str,
    url: &str,
    headers: &[KeyValuePair],
    body: Option<&str>,
) -> Result<HttpResponse, Error> {
    let client = reqwest::Client::new();
    let mut builder = match method.to_uppercase().as_str() {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        "PATCH" => client.patch(url),
        "HEAD" => client.head(url),
        other => {
            return Err(Error::msg(format!("unsupported HTTP method: {other}")));
        },
    };

    for kv in headers {
        builder = builder.header(&kv.key, &kv.value);
    }

    if let Some(b) = body {
        builder = builder.body(b.to_string());
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| Error::msg(format!("HTTP request failed: {e}")))?;

    let status = resp.status().as_u16();
    let resp_headers: Vec<KeyValuePair> = resp
        .headers()
        .iter()
        .map(|(k, v)| KeyValuePair {
            key: k.to_string(),
            value: v.to_str().unwrap_or("").to_string(),
        })
        .collect();
    let resp_body = resp
        .text()
        .await
        .map_err(|e| Error::msg(format!("failed to read HTTP response body: {e}")))?;

    Ok(HttpResponse {
        status,
        headers: resp_headers,
        body: resp_body,
    })
}

// ---------------------------------------------------------------------------
// Workspace path resolution
// ---------------------------------------------------------------------------

/// Resolve a plugin-provided path relative to the workspace root and verify
/// it does not escape the workspace boundary.
///
/// - Relative paths are joined onto `workspace_root`
/// - Absolute paths are used as-is
/// - The resulting canonical path must start with the canonical workspace root
fn resolve_within_workspace(
    workspace_root: &Path,
    requested: &str,
) -> Result<std::path::PathBuf, Error> {
    // Canonicalize the root first so all comparisons use the real path.
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());

    let requested_path = Path::new(requested);
    // Always join relative to canonical root for consistent comparison.
    let joined = if requested_path.is_absolute() {
        requested_path.to_path_buf()
    } else {
        canonical_root.join(requested_path)
    };

    // For existing paths, canonicalize fully (resolves symlinks).
    // For non-existing paths, canonicalize the parent if possible.
    let canonical_path = if joined.exists() {
        joined
            .canonicalize()
            .map_err(|e| Error::msg(format!("failed to resolve path: {e}")))?
    } else {
        // Canonicalize parent directory if it exists, then append the filename.
        let parent = joined.parent().unwrap_or(&joined);
        let filename = joined.file_name();
        if parent.exists() {
            let canonical_parent = parent
                .canonicalize()
                .map_err(|e| Error::msg(format!("failed to resolve parent: {e}")))?;
            match filename {
                Some(name) => canonical_parent.join(name),
                None => canonical_parent,
            }
        } else {
            // Neither path nor parent exist — do a lexical check.
            // The write will fail anyway if the directory doesn't exist.
            lexical_normalize(&joined)
        }
    };

    if !canonical_path.starts_with(&canonical_root) {
        return Err(Error::msg(format!(
            "path escapes workspace boundary: {requested} resolves to {}",
            canonical_path.display()
        )));
    }

    Ok(canonical_path)
}

/// Lexically normalize a path (resolve `.` and `..` without filesystem access).
fn lexical_normalize(path: &Path) -> std::path::PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                if !components.is_empty() {
                    components.pop();
                }
            },
            std::path::Component::CurDir => {},
            other => components.push(other),
        }
    }
    components.iter().collect()
}

// ---------------------------------------------------------------------------
// Host function registration helper
// ---------------------------------------------------------------------------

/// Register all host functions with an Extism `PluginBuilder`.
///
/// Registers:
/// - 7 host functions in the `extism:host/user` namespace (astrid_*)
/// - 3 shim functions in the `shim` namespace (for `QuickJS` kernel dispatch)
pub fn register_host_functions(
    builder: extism::PluginBuilder,
    user_data: UserData<HostState>,
) -> extism::PluginBuilder {
    use extism::ValType;

    builder
        // ── extism:host/user namespace (standard host functions) ──
        .with_function(
            "astrid_log",
            [PTR, PTR],
            [],
            user_data.clone(),
            astrid_log_impl,
        )
        .with_function(
            "astrid_get_config",
            [PTR],
            [PTR],
            user_data.clone(),
            astrid_get_config_impl,
        )
        .with_function(
            "astrid_kv_get",
            [PTR],
            [PTR],
            user_data.clone(),
            astrid_kv_get_impl,
        )
        .with_function(
            "astrid_kv_set",
            [PTR, PTR],
            [],
            user_data.clone(),
            astrid_kv_set_impl,
        )
        .with_function(
            "astrid_read_file",
            [PTR],
            [PTR],
            user_data.clone(),
            astrid_read_file_impl,
        )
        .with_function(
            "astrid_write_file",
            [PTR, PTR],
            [],
            user_data.clone(),
            astrid_write_file_impl,
        )
        .with_function(
            "astrid_http_request",
            [PTR],
            [PTR],
            user_data.clone(),
            astrid_http_request_impl,
        )
        // ── shim namespace (QuickJS kernel dispatch layer) ──
        //
        // The QuickJS kernel imports 3 functions from the `shim` namespace to
        // handle host function type introspection and dispatch. These are
        // normally provided by a generated shim WASM merged via wasm-merge.
        // We provide them as host functions instead, eliminating the merge step.
        //
        // Host function indices (alphabetically sorted):
        //   0: astrid_get_config   (PTR) -> PTR
        //   1: astrid_http_request (PTR) -> PTR
        //   2: astrid_kv_get       (PTR) -> PTR
        //   3: astrid_kv_set       (PTR, PTR) -> void
        //   4: astrid_log          (PTR, PTR) -> void
        //   5: astrid_read_file    (PTR) -> PTR
        //   6: astrid_write_file   (PTR, PTR) -> void
        .with_function_in_namespace(
            "shim",
            "__get_function_arg_type",
            [ValType::I32, ValType::I32],
            [ValType::I32],
            UserData::new(()),
            shim_get_function_arg_type,
        )
        .with_function_in_namespace(
            "shim",
            "__get_function_return_type",
            [ValType::I32],
            [ValType::I32],
            UserData::new(()),
            shim_get_function_return_type,
        )
        .with_function_in_namespace(
            "shim",
            "__invokeHostFunc",
            [
                ValType::I32,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I64,
            ],
            [ValType::I64],
            user_data,
            shim_invoke_host_func,
        )
}

// ---------------------------------------------------------------------------
// QuickJS shim functions (shim:: namespace)
// ---------------------------------------------------------------------------

/// Type codes used by the `QuickJS` kernel for host function dispatch.
const TYPE_VOID: i32 = 0;
const TYPE_I64: i32 = 2;

/// Number of host functions.
const NUM_HOST_FNS: i32 = 7;

/// Number of arguments per host function (alphabetically sorted).
///
/// `[get_config=1, http_request=1, kv_get=1, kv_set=2, log=2, read_file=1, write_file=2]`
const HOST_FN_ARG_COUNTS: [i32; 7] = [1, 1, 1, 2, 2, 1, 2];

/// Return type per host function: 0=void, 2=i64.
///
/// `[get_config→i64, http_request→i64, kv_get→i64, kv_set→void, log→void, read_file→i64, write_file→void]`
const HOST_FN_RETURN_TYPES: [i32; 7] = [
    TYPE_I64, TYPE_I64, TYPE_I64, TYPE_VOID, TYPE_VOID, TYPE_I64, TYPE_VOID,
];

/// `shim::__get_function_arg_type(func_idx, arg_idx) -> type_code`
///
/// Returns the WASM type code for a host function argument.
/// All our host functions use i64 (memory offset) arguments.
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn shim_get_function_arg_type(
    _plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    _user_data: UserData<()>,
) -> Result<(), Error> {
    let func_idx = inputs[0].unwrap_i32();
    let arg_idx = inputs[1].unwrap_i32();

    #[allow(clippy::cast_sign_loss)]
    let type_code = if (0..NUM_HOST_FNS).contains(&func_idx)
        && (0..HOST_FN_ARG_COUNTS[func_idx as usize]).contains(&arg_idx)
    {
        TYPE_I64 // All our args are i64 (memory offsets)
    } else {
        TYPE_VOID
    };

    outputs[0] = Val::I32(type_code);
    Ok(())
}

/// `shim::__get_function_return_type(func_idx) -> type_code`
///
/// Returns the WASM type code for a host function's return value.
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn shim_get_function_return_type(
    _plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    _user_data: UserData<()>,
) -> Result<(), Error> {
    let func_idx = inputs[0].unwrap_i32();

    #[allow(clippy::cast_sign_loss)]
    let type_code = if (0..NUM_HOST_FNS).contains(&func_idx) {
        HOST_FN_RETURN_TYPES[func_idx as usize]
    } else {
        TYPE_VOID
    };

    outputs[0] = Val::I32(type_code);
    Ok(())
}

/// `shim::__invokeHostFunc(func_idx, arg0, arg1, arg2, arg3, arg4) -> i64`
///
/// Dispatches a host function call from the `QuickJS` kernel.
/// Arguments are passed as i64 bit patterns (memory offsets for our functions).
#[allow(clippy::needless_pass_by_value)]
fn shim_invoke_host_func(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let func_idx = inputs[0].unwrap_i32();
    let args = &inputs[1..]; // arg0..arg4 as i64

    // Dispatch based on alphabetically sorted function index.
    // Each branch repackages i64 args as Val::I64 and delegates to the
    // actual host function implementation.
    match func_idx {
        0 => {
            // astrid_get_config(PTR) -> PTR
            let fn_inputs = [Val::I64(args[0].unwrap_i64())];
            let mut fn_outputs = [Val::I64(0)];
            astrid_get_config_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
        },
        1 => {
            // astrid_http_request(PTR) -> PTR
            let fn_inputs = [Val::I64(args[0].unwrap_i64())];
            let mut fn_outputs = [Val::I64(0)];
            astrid_http_request_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
        },
        2 => {
            // astrid_kv_get(PTR) -> PTR
            let fn_inputs = [Val::I64(args[0].unwrap_i64())];
            let mut fn_outputs = [Val::I64(0)];
            astrid_kv_get_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
        },
        3 => {
            // astrid_kv_set(PTR, PTR) -> void
            let fn_inputs = [
                Val::I64(args[0].unwrap_i64()),
                Val::I64(args[1].unwrap_i64()),
            ];
            let mut fn_outputs = [];
            astrid_kv_set_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(0);
        },
        4 => {
            // astrid_log(PTR, PTR) -> void
            let fn_inputs = [
                Val::I64(args[0].unwrap_i64()),
                Val::I64(args[1].unwrap_i64()),
            ];
            let mut fn_outputs = [];
            astrid_log_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(0);
        },
        5 => {
            // astrid_read_file(PTR) -> PTR
            let fn_inputs = [Val::I64(args[0].unwrap_i64())];
            let mut fn_outputs = [Val::I64(0)];
            astrid_read_file_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(fn_outputs[0].unwrap_i64());
        },
        6 => {
            // astrid_write_file(PTR, PTR) -> void
            let fn_inputs = [
                Val::I64(args[0].unwrap_i64()),
                Val::I64(args[1].unwrap_i64()),
            ];
            let mut fn_outputs = [];
            astrid_write_file_impl(plugin, &fn_inputs, &mut fn_outputs, user_data)?;
            outputs[0] = Val::I64(0);
        },
        _ => {
            outputs[0] = Val::I64(0);
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_boundary_relative_within() {
        let root = std::env::temp_dir();
        let result = resolve_within_workspace(&root, "subdir/file.txt");
        assert!(result.is_ok());
    }

    #[test]
    fn workspace_boundary_traversal_rejected() {
        let root = std::env::temp_dir().join("fake-workspace");
        let _ = std::fs::create_dir_all(&root);
        let result = resolve_within_workspace(&root, "../../etc/passwd");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("escapes workspace boundary"), "got: {err}");
    }

    #[test]
    fn lexical_normalize_removes_dotdot() {
        let normalized = lexical_normalize(Path::new("/a/b/../c/./d"));
        assert_eq!(normalized, Path::new("/a/c/d"));
    }

    #[test]
    fn lexical_normalize_handles_only_dots() {
        let normalized = lexical_normalize(Path::new("./foo/./bar"));
        assert_eq!(normalized, Path::new("foo/bar"));
    }

    /// Verify host function metadata is in strict alphabetical order.
    ///
    /// The shim dispatch layer, `HOST_FN_ARG_COUNTS`, `HOST_FN_RETURN_TYPES`,
    /// and the `shim_invoke_host_func` match arms ALL depend on alphabetical
    /// ordering. This test catches any desynchronization.
    #[test]
    fn host_function_ordering_is_alphabetical() {
        // Canonical alphabetically sorted host function names.
        // This list is the single source of truth — if a function is added,
        // it must be inserted here in sorted order.
        let expected_order = [
            "astrid_get_config",
            "astrid_http_request",
            "astrid_kv_get",
            "astrid_kv_set",
            "astrid_log",
            "astrid_read_file",
            "astrid_write_file",
        ];

        // Verify count matches constants
        assert_eq!(
            expected_order.len() as i32,
            NUM_HOST_FNS,
            "NUM_HOST_FNS doesn't match expected function count"
        );
        assert_eq!(
            HOST_FN_ARG_COUNTS.len(),
            expected_order.len(),
            "HOST_FN_ARG_COUNTS length mismatch"
        );
        assert_eq!(
            HOST_FN_RETURN_TYPES.len(),
            expected_order.len(),
            "HOST_FN_RETURN_TYPES length mismatch"
        );

        // Verify the list is actually sorted
        let mut sorted = expected_order;
        sorted.sort();
        assert_eq!(
            expected_order, sorted,
            "host function names must be alphabetically sorted"
        );

        // Verify arg counts match expected signatures:
        //   get_config(key)=1, http_request(json)=1, kv_get(key)=1,
        //   kv_set(key,val)=2, log(level,msg)=2, read_file(path)=1,
        //   write_file(path,content)=2
        let expected_args = [1, 1, 1, 2, 2, 1, 2];
        assert_eq!(
            HOST_FN_ARG_COUNTS, expected_args,
            "HOST_FN_ARG_COUNTS doesn't match expected signatures"
        );

        // Verify return types match:
        //   get_config→i64, http_request→i64, kv_get→i64,
        //   kv_set→void, log→void, read_file→i64, write_file→void
        let expected_returns = [
            TYPE_I64, TYPE_I64, TYPE_I64, TYPE_VOID, TYPE_VOID, TYPE_I64, TYPE_VOID,
        ];
        assert_eq!(
            HOST_FN_RETURN_TYPES, expected_returns,
            "HOST_FN_RETURN_TYPES doesn't match expected signatures"
        );
    }
}
