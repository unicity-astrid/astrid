//! Extism host function implementations matching the WIT `host` interface.
//!
//! Seven host functions are registered with every Extism plugin instance:
//!
//! | Function | Security Gate | Async Bridge |
//! |----------|--------------|--------------|
//! | `astralis_log` | No | No |
//! | `astralis_http_request` | Yes | Yes |
//! | `astralis_read_file` | Yes | Yes |
//! | `astralis_write_file` | Yes | Yes |
//! | `astralis_kv_get` | No | Yes |
//! | `astralis_kv_set` | No | Yes |
//! | `astralis_get_config` | No | No |
//!
//! All host functions use `UserData<HostState>` for shared state access.
//! Async operations are bridged via `Handle::block_on()` — this requires
//! the **multi-threaded** tokio runtime.

use std::path::Path;

use extism::{CurrentPlugin, Error, PTR, UserData, Val};

#[cfg(feature = "http")]
use astralis_core::plugin_abi::HttpResponse;
use astralis_core::plugin_abi::{KeyValuePair, LogLevel};

use super::host_state::HostState;

// ---------------------------------------------------------------------------
// astralis_log(level, message)
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astralis_log_impl(
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
// astralis_get_config(key) -> value_json
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astralis_get_config_impl(
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
// astralis_kv_get(key) -> value
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astralis_kv_get_impl(
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
// astralis_kv_set(key, value)
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astralis_kv_set_impl(
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
// astralis_read_file(path) -> content
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astralis_read_file_impl(
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
// astralis_write_file(path, content)
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astralis_write_file_impl(
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
// astralis_http_request(request_json) -> response_json
// ---------------------------------------------------------------------------

#[allow(clippy::needless_pass_by_value)] // Signature required by Extism callback API
fn astralis_http_request_impl(
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
            "HTTP support not enabled — enable the 'http' feature on astralis-plugins",
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
/// This is the single point where all 7 host functions are wired up.
pub fn register_host_functions(
    builder: extism::PluginBuilder,
    user_data: UserData<HostState>,
) -> extism::PluginBuilder {
    builder
        .with_function(
            "astralis_log",
            [PTR, PTR],
            [],
            user_data.clone(),
            astralis_log_impl,
        )
        .with_function(
            "astralis_get_config",
            [PTR],
            [PTR],
            user_data.clone(),
            astralis_get_config_impl,
        )
        .with_function(
            "astralis_kv_get",
            [PTR],
            [PTR],
            user_data.clone(),
            astralis_kv_get_impl,
        )
        .with_function(
            "astralis_kv_set",
            [PTR, PTR],
            [],
            user_data.clone(),
            astralis_kv_set_impl,
        )
        .with_function(
            "astralis_read_file",
            [PTR],
            [PTR],
            user_data.clone(),
            astralis_read_file_impl,
        )
        .with_function(
            "astralis_write_file",
            [PTR, PTR],
            [],
            user_data.clone(),
            astralis_write_file_impl,
        )
        .with_function(
            "astralis_http_request",
            [PTR],
            [PTR],
            user_data,
            astralis_http_request_impl,
        )
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
}
