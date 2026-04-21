//! Host function implementations for the `elicit` lifecycle API.
//!
//! These functions are called by WASM guests during `#[install]` or `#[upgrade]`
//! hooks to interactively collect user input (secrets, text, selections, arrays).

use crate::engine::wasm::bindings::astrid::capsule::elicit;
use crate::engine::wasm::bindings::astrid::capsule::types::ElicitRequest;
use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
use astrid_events::AstridEvent;
use astrid_events::ipc::{IpcMessage, IpcPayload, OnboardingField, OnboardingFieldType};
use uuid::Uuid;

/// Maximum timeout for interactive elicitation (120 seconds).
const MAX_ELICIT_TIMEOUT_MS: u64 = 120_000;

/// Map the SDK's string-typed request into the `OnboardingField` schema
/// used by the IPC layer and TUI.
fn map_to_onboarding_field(req: &ElicitRequest) -> Result<OnboardingField, String> {
    let field_type = match req.elicit_type.as_str() {
        "text" => OnboardingFieldType::Text,
        "secret" => OnboardingFieldType::Secret,
        "select" => {
            let options = req
                .options
                .as_ref()
                .filter(|o| !o.is_empty())
                .ok_or_else(|| "select elicit request requires non-empty options".to_string())?;
            OnboardingFieldType::Enum(options.clone())
        },
        "array" => OnboardingFieldType::Array,
        other => return Err(format!("unknown elicit type: {other}")),
    };

    Ok(OnboardingField {
        key: req.key.clone(),
        prompt: if req.description.is_empty() {
            req.key.clone()
        } else {
            req.description.clone()
        },
        description: if req.description.is_empty() {
            None
        } else {
            Some(req.description.clone())
        },
        field_type,
        default: req.default_value.clone(),
        placeholder: None,
    })
}

impl elicit::Host for HostState {
    /// Host function: `elicit(request) -> response_json`
    ///
    /// Blocks the WASM thread until the frontend (TUI or CLI) collects user input
    /// and publishes an `ElicitResponse` on the response topic.
    ///
    /// Only callable during a lifecycle phase (install/upgrade). Returns an error
    /// if called during normal runtime.
    fn elicit(&mut self, request: ElicitRequest) -> Result<String, String> {
        let field = map_to_onboarding_field(&request)?;
        let request_id = Uuid::new_v4();
        let response_topic = format!("astrid.v1.elicit.response.{request_id}");

        // Gate: elicit is only allowed during lifecycle hooks
        if self.lifecycle_phase.is_none() {
            return Err(
                "elicit is only available during #[install] or #[upgrade] lifecycle hooks"
                    .to_string(),
            );
        }

        // Subscribe to the response topic BEFORE publishing the request
        // to prevent a race where the response arrives before we're listening.
        let mut receiver = self.event_bus.subscribe_topic(&response_topic);

        let runtime_handle = self.runtime_handle.clone();
        let event_bus = self.event_bus.clone();
        let capsule_id = self.capsule_id.to_string();
        let secret_store = self.effective_secret_store().clone();
        let cancel_token = self.cancel_token.clone();
        let host_semaphore = self.host_semaphore.clone();

        // Publish the elicit request to the event bus
        let request_payload = IpcPayload::ElicitRequest {
            request_id,
            capsule_id: capsule_id.clone(),
            field,
        };
        let message = IpcMessage::new(
            "astrid.v1.elicit",
            request_payload,
            Uuid::nil(), // Kernel-originated
        );
        event_bus.publish(AstridEvent::Ipc {
            message,
            metadata: astrid_events::EventMetadata::default(),
        });

        tracing::debug!(
            capsule = %capsule_id,
            key = %request.key,
            kind = %request.elicit_type,
            %request_id,
            "Published elicit request, waiting for response"
        );

        // Block the WASM thread until a response arrives, timeout expires, or
        // the capsule is unloaded (cancellation). Routed through the host
        // semaphore to bound concurrent blocking operations across all capsules.
        //
        // Note: the helper uses a biased select that strictly prioritises
        // cancellation over completion. If a response arrives in the same poll
        // tick as cancellation, the response is discarded. This is acceptable
        // during teardown and prevents delayed shutdown under high throughput.
        let event = util::bounded_block_on_cancellable(
            &runtime_handle,
            &host_semaphore,
            &cancel_token,
            async {
                tokio::time::timeout(
                    std::time::Duration::from_millis(MAX_ELICIT_TIMEOUT_MS),
                    receiver.recv(),
                )
                .await
                .ok()
                .flatten()
            },
        )
        .flatten();

        // Extract the response
        let response_json = match event {
            Some(event) => {
                if let AstridEvent::Ipc { message, .. } = &*event {
                    match &message.payload {
                        IpcPayload::ElicitResponse { value, values, .. } => {
                            // Detect cancellation: both value and values are None
                            if value.is_none() && values.is_none() {
                                return Err("user cancelled elicit request".to_string());
                            }

                            // Build response JSON matching what the SDK expects
                            match request.elicit_type.as_str() {
                                "secret" => {
                                    // Persist the secret via the SecretStore abstraction.
                                    // This uses the OS keychain when available, falling
                                    // back to KV storage in headless/CI environments.
                                    let secret_val = value.clone().unwrap_or_default();
                                    if secret_val.is_empty() {
                                        return Err(
                                            "received empty secret value from elicit response"
                                                .to_string(),
                                        );
                                    }
                                    secret_store
                                        .set(&request.key, &secret_val)
                                        .map_err(|e| format!("failed to persist secret: {e}"))?;

                                    // Secret: SDK expects {"ok": true}
                                    serde_json::to_string(&serde_json::json!({"ok": true}))
                                        .map_err(|e| format!("failed to serialize response: {e}"))?
                                },
                                "array" => {
                                    // Array: SDK expects {"values": [...]}
                                    let vals = values.clone().unwrap_or_default();
                                    serde_json::to_string(&serde_json::json!({"values": vals}))
                                        .map_err(|e| format!("failed to serialize response: {e}"))?
                                },
                                _ => {
                                    // Text/Select: SDK expects {"value": "..."}
                                    let val = value.clone().unwrap_or_default();
                                    serde_json::to_string(&serde_json::json!({"value": val}))
                                        .map_err(|e| format!("failed to serialize response: {e}"))?
                                },
                            }
                        },
                        _ => {
                            return Err(
                                "unexpected IPC payload type in elicit response".to_string()
                            );
                        },
                    }
                } else {
                    return Err("unexpected event type in elicit response".to_string());
                }
            },
            None => {
                // Timeout expired, capsule unloading (cancellation), or channel closed.
                return Err(
                    "elicit request timed out, was cancelled, or response channel closed"
                        .to_string(),
                );
            },
        };

        Ok(response_json)
    }

    /// Host function: `has_secret(key) -> bool`
    ///
    /// Checks whether a secret key has been stored for this capsule.
    /// Uses the [`SecretStore`] abstraction (OS keychain with KV fallback).
    fn has_secret(&mut self, key: String) -> Result<bool, String> {
        self.effective_secret_store()
            .exists(&key)
            .map_err(|e| format!("failed to check for secret: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_elicit_request(
        kind: &str,
        key: &str,
        description: &str,
        options: Option<Vec<String>>,
        default: Option<String>,
    ) -> ElicitRequest {
        ElicitRequest {
            elicit_type: kind.to_string(),
            key: key.to_string(),
            description: description.to_string(),
            options,
            default_value: default,
        }
    }

    #[test]
    fn map_text_request() {
        let req = make_elicit_request(
            "text",
            "api_url",
            "Enter API URL",
            None,
            Some("https://example.com".into()),
        );
        let field = map_to_onboarding_field(&req).unwrap();
        assert_eq!(field.key, "api_url");
        assert_eq!(field.field_type, OnboardingFieldType::Text);
        assert_eq!(field.default.as_deref(), Some("https://example.com"));
        assert_eq!(field.prompt, "Enter API URL");
    }

    #[test]
    fn map_secret_request() {
        let req = make_elicit_request("secret", "api_key", "Enter your API key", None, None);
        let field = map_to_onboarding_field(&req).unwrap();
        assert_eq!(field.field_type, OnboardingFieldType::Secret);
    }

    #[test]
    fn map_select_request() {
        let req = make_elicit_request(
            "select",
            "network",
            "Choose network",
            Some(vec!["mainnet".into(), "testnet".into()]),
            None,
        );
        let field = map_to_onboarding_field(&req).unwrap();
        assert_eq!(
            field.field_type,
            OnboardingFieldType::Enum(vec!["mainnet".into(), "testnet".into()])
        );
    }

    #[test]
    fn map_select_request_empty_options_fails() {
        let req = make_elicit_request("select", "network", "", Some(vec![]), None);
        let err = map_to_onboarding_field(&req).unwrap_err();
        assert!(err.contains("non-empty options"));
    }

    #[test]
    fn map_select_request_no_options_fails() {
        let req = make_elicit_request("select", "network", "", None, None);
        let err = map_to_onboarding_field(&req).unwrap_err();
        assert!(err.contains("non-empty options"));
    }

    #[test]
    fn map_array_request() {
        let req = make_elicit_request("array", "relays", "Enter relay URLs", None, None);
        let field = map_to_onboarding_field(&req).unwrap();
        assert_eq!(field.field_type, OnboardingFieldType::Array);
    }

    #[test]
    fn map_unknown_type_fails() {
        let req = make_elicit_request("checkbox", "foo", "", None, None);
        let err = map_to_onboarding_field(&req).unwrap_err();
        assert!(err.contains("unknown elicit type"));
    }

    #[test]
    fn map_text_uses_key_as_prompt_when_no_description() {
        let req = make_elicit_request("text", "my_setting", "", None, None);
        let field = map_to_onboarding_field(&req).unwrap();
        assert_eq!(field.prompt, "my_setting");
        assert!(field.description.is_none());
    }
}

// ---------------------------------------------------------------------------
// Chain tests: drive `has_secret` synchronously on a HostState with manually-
// installed invocation fields. Verifies `effective_secret_store()` wiring: a
// key set via the invocation store must not be visible via the load-time
// store and vice versa. Mirrors the pattern established in `host/fs.rs` for
// per-invocation VFS re-scoping (#549).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod secret_chain_tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tokio::sync::Semaphore;
    use tokio_util::sync::CancellationToken;

    use crate::capsule::CapsuleId;
    use crate::engine::wasm::bindings::astrid::capsule::elicit::Host as ElicitHost;
    use crate::engine::wasm::host::process::ProcessTracker;
    use crate::engine::wasm::host_state::HostState;
    use astrid_storage::ScopedKvStore;
    use astrid_storage::secret::SecretStore;

    /// Build a HostState carrying an owner-scoped secret store. Each call to
    /// this helper returns a fresh state with an independent in-memory KV.
    fn make_host_state_with_secret(
        rt: tokio::runtime::Handle,
        owner_namespace: &str,
    ) -> (HostState, Arc<dyn SecretStore>) {
        let kv_store = Arc::new(astrid_storage::MemoryKvStore::new());
        let kv = ScopedKvStore::new(kv_store, owner_namespace).unwrap();
        let owner_secret: Arc<dyn SecretStore> =
            Arc::new(astrid_storage::KvSecretStore::new(kv.clone(), rt.clone()));

        let state = HostState {
            wasi_ctx: wasmtime_wasi::WasiCtxBuilder::new().build(),
            resource_table: wasmtime::component::ResourceTable::new(),
            store_limits: wasmtime::StoreLimitsBuilder::new().build(),
            principal: astrid_core::PrincipalId::default(),
            capsule_uuid: uuid::Uuid::new_v4(),
            caller_context: None,
            invocation_kv: None,
            capsule_log: None,
            capsule_id: CapsuleId::from_static("test-capsule"),
            workspace_root: std::path::PathBuf::from("/tmp"),
            vfs: Arc::new(astrid_vfs::HostVfs::new()),
            vfs_root_handle: astrid_capabilities::DirHandle::new(),
            home: None,
            tmp: None,
            invocation_home: None,
            invocation_tmp: None,
            invocation_secret_store: None,
            invocation_capsule_log: None,
            overlay_vfs: None,
            upper_dir: None,
            kv,
            event_bus: astrid_events::EventBus::with_capacity(128),
            ipc_limiter: astrid_events::ipc::IpcRateLimiter::new(),
            subscriptions: HashMap::new(),
            next_subscription_id: 1,
            config: HashMap::new(),
            ipc_publish_patterns: Vec::new(),
            ipc_subscribe_patterns: Vec::new(),
            security: None,
            hook_manager: None,
            capsule_registry: None,
            runtime_handle: rt,
            has_uplink_capability: false,
            inbound_tx: None,
            registered_uplinks: Vec::new(),
            cli_socket_listener: None,
            active_streams: HashMap::new(),
            next_stream_id: 1,
            active_http_streams: HashMap::new(),
            next_http_stream_id: 1,
            lifecycle_phase: None,
            secret_store: Arc::clone(&owner_secret),
            ready_tx: None,
            host_semaphore: Arc::new(Semaphore::new(2)),
            cancel_token: CancellationToken::new(),
            session_token: None,
            interceptor_handles: Vec::new(),
            allowance_store: None,
            identity_store: None,
            background_processes: HashMap::new(),
            next_process_id: 1,
            process_tracker: Arc::new(ProcessTracker::new()),
        };
        (state, owner_secret)
    }

    fn make_invocation_store(rt: tokio::runtime::Handle, namespace: &str) -> Arc<dyn SecretStore> {
        let kv_store = Arc::new(astrid_storage::MemoryKvStore::new());
        let kv = ScopedKvStore::new(kv_store, namespace).unwrap();
        Arc::new(astrid_storage::KvSecretStore::new(kv, rt))
    }

    /// Drive a closure in a blocking context so KvSecretStore's internal
    /// `Handle::block_on` works — same sync/async bridge pattern as
    /// production host functions.
    async fn blocking<T, F>(f: F) -> T
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        tokio::task::spawn_blocking(f)
            .await
            .expect("spawn_blocking join")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn has_secret_reads_invocation_store_when_installed() {
        let rt = tokio::runtime::Handle::current();
        let (mut state, owner_secret) =
            make_host_state_with_secret(rt.clone(), "capsule:test-owner");
        let alice_secret = make_invocation_store(rt, "capsule:test-alice");

        // Owner has `shared_key`; Alice does not.
        {
            let s = Arc::clone(&owner_secret);
            blocking(move || s.set("shared_key", "owner-val").unwrap()).await;
        }
        state.invocation_secret_store = Some(Arc::clone(&alice_secret));

        // Via the accessor, `has_secret` consults Alice's store — the owner's
        // entry is not visible.
        let (state, got) = blocking(move || {
            let mut s = state;
            let got = s.has_secret("shared_key".to_string()).unwrap();
            (s, got)
        })
        .await;
        assert!(!got, "invocation store is empty; owner's key must not leak");

        // Alice sets her own; owner's view is unchanged.
        {
            let s = Arc::clone(&alice_secret);
            blocking(move || s.set("shared_key", "alice-val").unwrap()).await;
        }
        let (mut state, got) = blocking(move || {
            let mut s = state;
            let got = s.has_secret("shared_key".to_string()).unwrap();
            (s, got)
        })
        .await;
        assert!(got);

        // Drop invocation context: falls back to owner's store.
        state.invocation_secret_store = None;
        let (_state, got) = blocking(move || {
            let mut s = state;
            let got = s.has_secret("shared_key".to_string()).unwrap();
            (s, got)
        })
        .await;
        assert!(got, "owner's key still present after clear");

        // Sanity: owner never saw Alice's value.
        let (owner_val, alice_val) = blocking(move || {
            (
                owner_secret.get("shared_key").unwrap(),
                alice_secret.get("shared_key").unwrap(),
            )
        })
        .await;
        assert_eq!(owner_val.as_deref(), Some("owner-val"));
        assert_eq!(alice_val.as_deref(), Some("alice-val"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn has_secret_falls_back_to_load_time_store() {
        // Regression guard: single-tenant path (no invocation store installed)
        // must see load-time secrets.
        let rt = tokio::runtime::Handle::current();
        let (state, owner_secret) = make_host_state_with_secret(rt, "capsule:test-owner");
        {
            let s = Arc::clone(&owner_secret);
            blocking(move || s.set("api_key", "sk-load").unwrap()).await;
        }
        assert!(state.invocation_secret_store.is_none());
        let (_state, got1, got2) = blocking(move || {
            let mut state = state;
            let got1 = state.has_secret("api_key".to_string()).unwrap();
            let got2 = state.has_secret("other_key".to_string()).unwrap();
            (state, got1, got2)
        })
        .await;
        assert!(got1);
        assert!(!got2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn has_secret_isolates_across_sequential_invocations() {
        // Same HostState, invocation store swapped between calls — each call
        // sees only the currently-installed principal's secrets.
        let rt = tokio::runtime::Handle::current();
        let (mut state, _owner_secret) =
            make_host_state_with_secret(rt.clone(), "capsule:test-owner");

        let alice_secret = make_invocation_store(rt.clone(), "capsule:test-alice");
        let bob_secret = make_invocation_store(rt, "capsule:test-bob");
        {
            let a = Arc::clone(&alice_secret);
            let b = Arc::clone(&bob_secret);
            blocking(move || {
                a.set("pk", "alice-pk").unwrap();
                b.set("pk", "bob-pk").unwrap();
            })
            .await;
        }

        state.invocation_secret_store = Some(Arc::clone(&alice_secret));
        let (mut state, alice_view) = blocking(move || {
            let mut s = state;
            let v = s.has_secret("pk".to_string()).unwrap();
            (s, v)
        })
        .await;
        assert!(alice_view);
        state.invocation_secret_store = None;

        state.invocation_secret_store = Some(Arc::clone(&bob_secret));
        let (_state, bob_view) = blocking(move || {
            let mut s = state;
            let v = s.has_secret("pk".to_string()).unwrap();
            (s, v)
        })
        .await;
        assert!(bob_view);

        // Both isolated: each only sees its own key.
        let (a_val, b_val) = blocking(move || {
            (
                alice_secret.get("pk").unwrap(),
                bob_secret.get("pk").unwrap(),
            )
        })
        .await;
        assert_eq!(a_val.as_deref(), Some("alice-pk"));
        assert_eq!(b_val.as_deref(), Some("bob-pk"));
    }
}
