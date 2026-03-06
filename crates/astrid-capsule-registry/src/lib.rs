#![deny(unsafe_code)]
#![deny(clippy::all)]

//! LLM Provider Registry capsule.
//!
//! Discovers available LLM providers from loaded capsule manifests and
//! manages model selection. Frontends query this capsule to list models
//! and switch between them.
//!
//! # IPC Protocol
//!
//! **Queries** (publish to these topics, registry responds on `registry.response.*`):
//! - `registry.get_providers` — returns list of available LLM providers
//! - `registry.get_active_model` — returns the currently active model
//! - `registry.set_active_model` — payload: `{"model_id": "..."}`, sets active model
//!
//! **Events** (published by registry):
//! - `registry.active_model_changed` — payload: `ProviderEntry`, emitted on model change

use astrid_events::kernel_api::{
    CapsuleMetadataEntry, KernelRequest, KernelResponse, SYSTEM_SESSION_UUID,
};
use astrid_sdk::prelude::*;
use extism_pdk::FnResult;
use serde::{Deserialize, Serialize};

/// A resolved LLM provider with its IPC routing topics.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderEntry {
    /// Model ID from the capsule manifest (e.g. "claude-3-5-sonnet-20241022").
    id: String,
    /// Human-readable description.
    description: String,
    /// Capsule that provides this model.
    capsule: String,
    /// IPC topic to publish LLM requests to.
    request_topic: String,
    /// IPC topic the provider streams responses on.
    stream_topic: String,
    /// Model capabilities.
    capabilities: Vec<String>,
}

/// The persisted registry state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RegistryState {
    providers: Vec<ProviderEntry>,
    active_model_id: Option<String>,
}

const STATE_KEY: &str = "registry_state";

fn load_state() -> RegistryState {
    kv::get_json::<RegistryState>(STATE_KEY).unwrap_or_default()
}

fn save_state(state: &RegistryState) {
    let _ = kv::set_json(STATE_KEY, state);
}

/// Query the kernel for capsule metadata and resolve LLM providers.
fn discover_providers() -> Vec<ProviderEntry> {
    let req = KernelRequest::GetCapsuleMetadata;
    let val = match serde_json::to_value(req) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    // Publish the request and wait for the response
    if ipc::publish_json("kernel.request.get_capsule_metadata", &val).is_err() {
        return Vec::new();
    }

    // The response will come back on the event bus — but we're in a synchronous
    // WASM context. We need to poll for it.
    let sub = match ipc::subscribe("kernel.response.get_capsule_metadata") {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };

    // Poll with a yield between iterations to avoid busy-spinning.
    // The kernel router responds nearly instantly but we must not
    // consume CPU while waiting.
    for _ in 0..100 {
        if let Ok(bytes) = ipc::poll_bytes(&sub) {
            if !bytes.is_empty() {
                // Verify the response came from the kernel (source_id = system session UUID)
                // by checking the source_id field in the poll envelope messages.
                let _ = ipc::unsubscribe(&sub);
                return parse_metadata_response(&bytes);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = ipc::unsubscribe(&sub);
    Vec::new()
}

/// Parse the poll envelope and extract provider entries from the kernel response.
/// Only accepts messages whose `source_id` matches the kernel's system UUID.
fn parse_metadata_response(poll_bytes: &[u8]) -> Vec<ProviderEntry> {
    let envelope: serde_json::Value = match serde_json::from_slice(poll_bytes) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let messages = match envelope.get("messages").and_then(|m| m.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    for msg in messages {
        // Verify the message came from the kernel router (system session)
        let source = msg.get("source_id").and_then(|s| s.as_str()).unwrap_or("");
        if source != SYSTEM_SESSION_UUID {
            let _ = sys::log(
                "warn",
                format!("Ignoring metadata response from untrusted source: {source}"),
            );
            continue;
        }

        let payload = match msg.get("payload") {
            Some(p) => p,
            None => continue,
        };

        // The payload is IpcPayload::RawJson containing a KernelResponse
        let inner = match payload.get("data") {
            Some(d) => d,
            None => continue,
        };

        if let Ok(KernelResponse::CapsuleMetadata(entries)) =
            serde_json::from_value::<KernelResponse>(inner.clone())
        {
            return resolve_providers(&entries);
        }
    }
    Vec::new()
}

/// Convert capsule metadata entries into resolved provider entries.
fn resolve_providers(entries: &[CapsuleMetadataEntry]) -> Vec<ProviderEntry> {
    let mut providers = Vec::new();
    for entry in entries {
        for llm_def in &entry.llm_providers {
            // Derive the request topic from the capsule's interceptor events
            let request_topic = entry
                .interceptor_events
                .iter()
                .find(|e| e.starts_with("llm.request.generate"))
                .cloned()
                .unwrap_or_else(|| format!("llm.request.generate.{}", entry.name));

            let suffix = request_topic
                .strip_prefix("llm.request.generate.")
                .unwrap_or(&entry.name);
            let stream_topic = format!("llm.stream.{suffix}");

            providers.push(ProviderEntry {
                id: llm_def.id.clone(),
                description: llm_def.description.clone(),
                capsule: entry.name.clone(),
                request_topic,
                stream_topic,
                capabilities: llm_def.capabilities.clone(),
            });
        }
    }
    providers
}

/// Publish the active model changed event so the orchestrator (and frontends) can react.
fn publish_model_changed(provider: &ProviderEntry) {
    let _ = ipc::publish_json("registry.active_model_changed", provider);
}

/// Handle a `registry.get_providers` request.
fn handle_get_providers() {
    let providers = discover_providers();
    let mut state = load_state();

    // Only overwrite providers if discovery returned results.
    // An empty result (timeout, capsule not loaded) must not clobber
    // a previously known-good list, as that would break active_model_id
    // references and cause the TUI to show no models.
    if !providers.is_empty() {
        state.providers = providers;
        save_state(&state);
    } else if state.providers.is_empty() {
        let _ = sys::log(
            "warn",
            "Provider discovery returned empty and no cached providers exist",
        );
    }

    let _ = ipc::publish_json("registry.response.get_providers", &state.providers);
}

/// Handle a `registry.get_active_model` request.
fn handle_get_active_model() {
    let state = load_state();
    let active = state
        .active_model_id
        .as_ref()
        .and_then(|id| state.providers.iter().find(|p| &p.id == id));

    let _ = ipc::publish_json("registry.response.get_active_model", &active);
}

/// Handle a `registry.set_active_model` request.
fn handle_set_active_model(payload: &serde_json::Value) {
    let model_id = match payload.get("model_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            let _ = ipc::publish_json(
                "registry.response.set_active_model",
                &serde_json::json!({"error": "missing model_id"}),
            );
            return;
        },
    };

    let mut state = load_state();

    // Refresh providers if stale
    if state.providers.is_empty() {
        state.providers = discover_providers();
    }

    if let Some(provider) = state.providers.iter().find(|p| p.id == model_id).cloned() {
        state.active_model_id = Some(model_id);
        save_state(&state);
        publish_model_changed(&provider);
        let _ = ipc::publish_json(
            "registry.response.set_active_model",
            &serde_json::json!({"status": "ok", "active_model": provider}),
        );
    } else {
        let _ = ipc::publish_json(
            "registry.response.set_active_model",
            &serde_json::json!({"error": format!("unknown model: {model_id}")}),
        );
    }
}

/// Auto-select the sole provider if exactly one is available.
fn auto_select_if_single(state: &mut RegistryState) {
    if state.providers.len() == 1 && state.active_model_id.is_none() {
        let provider = state.providers[0].clone();
        state.active_model_id = Some(provider.id.clone());
        save_state(state);
        publish_model_changed(&provider);
        let _ = sys::log(
            "info",
            format!("Auto-selected sole LLM provider: {}", provider.id),
        );
    }
}

#[plugin_fn]
pub fn run() -> FnResult<()> {
    let _ = sys::log("info", "Registry capsule starting");

    let sub = ipc::subscribe("registry.*").map_err(|e| extism_pdk::Error::msg(e.to_string()))?;

    // Wait for the kernel to signal that all capsules have been loaded.
    // This is event-driven: the kernel publishes `kernel.capsules_loaded`
    // after `load_all_capsules()` completes, so we don't need arbitrary
    // sleeps or retry loops.
    let loaded_sub = ipc::subscribe("kernel.capsules_loaded")
        .map_err(|e| extism_pdk::Error::msg(e.to_string()))?;

    let mut capsules_ready = false;
    for _ in 0..500 {
        // 500 × 10ms = 5s max wait
        if let Ok(bytes) = ipc::poll_bytes(&loaded_sub) {
            if !bytes.is_empty() {
                capsules_ready = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = ipc::unsubscribe(&loaded_sub);

    if !capsules_ready {
        let _ = sys::log(
            "warn",
            "Timed out waiting for kernel.capsules_loaded — proceeding with discovery anyway",
        );
    }

    // Now that all capsules are loaded, discover providers.
    let providers = discover_providers();
    let mut state = load_state();
    state.providers = providers;
    save_state(&state);
    auto_select_if_single(&mut state);

    // Event loop
    loop {
        match ipc::poll_bytes(&sub) {
            Ok(bytes) => {
                if !bytes.is_empty() {
                    handle_poll_envelope(&bytes);
                }
            },
            Err(_) => break,
        }

        // Brief sleep to avoid busy-spinning
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    Ok(())
}

/// Parse the poll envelope and dispatch individual messages.
fn handle_poll_envelope(poll_bytes: &[u8]) {
    let envelope: serde_json::Value = match serde_json::from_slice(poll_bytes) {
        Ok(v) => v,
        Err(_) => return,
    };

    let messages = match envelope.get("messages").and_then(|m| m.as_array()) {
        Some(arr) => arr,
        None => return,
    };

    for msg in messages {
        let topic = match msg.get("topic").and_then(|t| t.as_str()) {
            Some(t) => t,
            None => continue,
        };

        // Skip our own response messages to avoid unnecessary processing.
        if topic.starts_with("registry.response.") || topic == "registry.active_model_changed" {
            continue;
        }

        match topic {
            "registry.get_providers" => handle_get_providers(),
            "registry.get_active_model" => handle_get_active_model(),
            "registry.set_active_model" => {
                if let Some(payload) = msg.get("payload") {
                    handle_set_active_model(payload);
                }
            },
            _ => {},
        }
    }
}
