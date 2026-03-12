//! Host function implementations for the `elicit` lifecycle API.
//!
//! These functions are called by WASM guests during `#[install]` or `#[upgrade]`
//! hooks to interactively collect user input (secrets, text, selections, arrays).

use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
use astrid_events::AstridEvent;
use astrid_events::ipc::{IpcMessage, IpcPayload, OnboardingField, OnboardingFieldType};
use extism::{CurrentPlugin, Error, UserData, Val};
use serde::Deserialize;
use uuid::Uuid;

/// Maximum timeout for interactive elicitation (120 seconds).
const MAX_ELICIT_TIMEOUT_MS: u64 = 120_000;

/// The wire format sent by the SDK's `elicit` module.
#[derive(Deserialize)]
struct GuestElicitRequest {
    #[serde(rename = "type")]
    kind: String,
    key: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    options: Option<Vec<String>>,
    #[serde(default)]
    default: Option<String>,
}

/// The wire format for `has_secret` requests from the SDK.
#[derive(Deserialize)]
struct GuestHasSecretRequest {
    key: String,
}

/// Map the SDK's string-typed request into the `OnboardingField` schema
/// used by the IPC layer and TUI.
fn map_to_onboarding_field(req: &GuestElicitRequest) -> Result<OnboardingField, Error> {
    let field_type = match req.kind.as_str() {
        "text" => OnboardingFieldType::Text,
        "secret" => OnboardingFieldType::Secret,
        "select" => {
            let options = req
                .options
                .as_ref()
                .filter(|o| !o.is_empty())
                .ok_or_else(|| Error::msg("select elicit request requires non-empty options"))?;
            OnboardingFieldType::Enum(options.clone())
        },
        "array" => OnboardingFieldType::Array,
        other => return Err(Error::msg(format!("unknown elicit type: {other}"))),
    };

    Ok(OnboardingField {
        key: req.key.clone(),
        prompt: req.description.as_ref().unwrap_or(&req.key).clone(),
        description: req.description.clone(),
        field_type,
        default: req.default.clone(),
        placeholder: None,
    })
}

/// Host function: `astrid_elicit(request_json) -> response_json`
///
/// Blocks the WASM thread until the frontend (TUI or CLI) collects user input
/// and publishes an `ElicitResponse` on the response topic.
///
/// Only callable during a lifecycle phase (install/upgrade). Returns an error
/// if called during normal runtime.
#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_elicit_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    // Parse the guest's JSON request
    let request_bytes = util::get_safe_bytes(plugin, &inputs[0], util::MAX_GUEST_PAYLOAD_LEN)?;
    let guest_req: GuestElicitRequest = serde_json::from_slice(&request_bytes)
        .map_err(|e| Error::msg(format!("invalid elicit request JSON: {e}")))?;

    let field = map_to_onboarding_field(&guest_req)?;
    let request_id = Uuid::new_v4();
    let response_topic = format!("astrid.v1.elicit.response.{request_id}");

    let ud = user_data.get()?;

    // Lock state: verify lifecycle phase, subscribe to response topic, extract
    // what we need, then drop the lock before blocking.
    let (mut receiver, runtime_handle, event_bus, capsule_id, secret_store, cancel_token) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

        // Gate: elicit is only allowed during lifecycle hooks
        if state.lifecycle_phase.is_none() {
            return Err(Error::msg(
                "elicit is only available during #[install] or #[upgrade] lifecycle hooks",
            ));
        }

        // Subscribe to the response topic BEFORE publishing the request
        // to prevent a race where the response arrives before we're listening.
        let receiver = state.event_bus.subscribe_topic(&response_topic);

        let runtime_handle = state.runtime_handle.clone();
        let event_bus = state.event_bus.clone();
        let capsule_id = state.capsule_id.to_string();
        let secret_store = state.secret_store.clone();
        let cancel_token = state.cancel_token.clone();

        (
            receiver,
            runtime_handle,
            event_bus,
            capsule_id,
            secret_store,
            cancel_token,
        )
    };

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
        key = %guest_req.key,
        kind = %guest_req.kind,
        %request_id,
        "Published elicit request, waiting for response"
    );

    // Block the WASM thread until a response arrives, timeout expires, or
    // the capsule is unloaded (cancellation).
    let event = runtime_handle.block_on(async {
        tokio::select! {
            result = tokio::time::timeout(
                std::time::Duration::from_millis(MAX_ELICIT_TIMEOUT_MS),
                receiver.recv(),
            ) => result.ok().flatten(),
            () = cancel_token.cancelled() => None,
        }
    });

    // Extract the response
    let response_json = match event {
        Some(event) => {
            if let AstridEvent::Ipc { message, .. } = &*event {
                match &message.payload {
                    IpcPayload::ElicitResponse { value, values, .. } => {
                        // Detect cancellation: both value and values are None
                        if value.is_none() && values.is_none() {
                            return Err(Error::msg("user cancelled elicit request"));
                        }

                        // Build response JSON matching what the SDK expects
                        match guest_req.kind.as_str() {
                            "secret" => {
                                // Persist the secret via the SecretStore abstraction.
                                // This uses the OS keychain when available, falling
                                // back to KV storage in headless/CI environments.
                                let secret_val = value.clone().unwrap_or_default();
                                if secret_val.is_empty() {
                                    return Err(Error::msg(
                                        "received empty secret value from elicit response",
                                    ));
                                }
                                secret_store.set(&guest_req.key, &secret_val).map_err(|e| {
                                    Error::msg(format!("failed to persist secret: {e}"))
                                })?;

                                // Secret: SDK expects {"ok": true}
                                serde_json::to_vec(&serde_json::json!({"ok": true})).map_err(
                                    |e| Error::msg(format!("failed to serialize response: {e}")),
                                )?
                            },
                            "array" => {
                                // Array: SDK expects {"values": [...]}
                                let vals = values.clone().unwrap_or_default();
                                serde_json::to_vec(&serde_json::json!({"values": vals})).map_err(
                                    |e| Error::msg(format!("failed to serialize response: {e}")),
                                )?
                            },
                            _ => {
                                // Text/Select: SDK expects {"value": "..."}
                                let val = value.clone().unwrap_or_default();
                                serde_json::to_vec(&serde_json::json!({"value": val})).map_err(
                                    |e| Error::msg(format!("failed to serialize response: {e}")),
                                )?
                            },
                        }
                    },
                    _ => {
                        return Err(Error::msg("unexpected IPC payload type in elicit response"));
                    },
                }
            } else {
                return Err(Error::msg("unexpected event type in elicit response"));
            }
        },
        None => {
            // Timeout, cancellation, or channel closed
            return Err(Error::msg(
                "elicit request timed out waiting for user input",
            ));
        },
    };

    let mem = plugin.memory_new(&response_json)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

/// Host function: `astrid_has_secret(request_json) -> response_json`
///
/// Checks whether a secret key has been stored for this capsule.
/// Uses the [`SecretStore`] abstraction (OS keychain with KV fallback).
#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_has_secret_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let request_bytes = util::get_safe_bytes(plugin, &inputs[0], util::MAX_KEY_LEN)?;
    let req: GuestHasSecretRequest = serde_json::from_slice(&request_bytes)
        .map_err(|e| Error::msg(format!("invalid has_secret request JSON: {e}")))?;

    let ud = user_data.get()?;

    // Extract secret store, then drop the lock.
    let secret_store = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;
        state.secret_store.clone()
    };

    let exists = secret_store
        .exists(&req.key)
        .map_err(|e| Error::msg(format!("failed to check for secret: {e}")))?;

    let response = serde_json::to_vec(&serde_json::json!({"exists": exists}))
        .map_err(|e| Error::msg(format!("failed to serialize has_secret response: {e}")))?;

    let mem = plugin.memory_new(&response)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_text_request() {
        let req = GuestElicitRequest {
            kind: "text".into(),
            key: "api_url".into(),
            description: Some("Enter API URL".into()),
            options: None,
            default: Some("https://example.com".into()),
        };
        let field = map_to_onboarding_field(&req).unwrap();
        assert_eq!(field.key, "api_url");
        assert_eq!(field.field_type, OnboardingFieldType::Text);
        assert_eq!(field.default.as_deref(), Some("https://example.com"));
        assert_eq!(field.prompt, "Enter API URL");
    }

    #[test]
    fn map_secret_request() {
        let req = GuestElicitRequest {
            kind: "secret".into(),
            key: "api_key".into(),
            description: Some("Enter your API key".into()),
            options: None,
            default: None,
        };
        let field = map_to_onboarding_field(&req).unwrap();
        assert_eq!(field.field_type, OnboardingFieldType::Secret);
    }

    #[test]
    fn map_select_request() {
        let req = GuestElicitRequest {
            kind: "select".into(),
            key: "network".into(),
            description: Some("Choose network".into()),
            options: Some(vec!["mainnet".into(), "testnet".into()]),
            default: None,
        };
        let field = map_to_onboarding_field(&req).unwrap();
        assert_eq!(
            field.field_type,
            OnboardingFieldType::Enum(vec!["mainnet".into(), "testnet".into()])
        );
    }

    #[test]
    fn map_select_request_empty_options_fails() {
        let req = GuestElicitRequest {
            kind: "select".into(),
            key: "network".into(),
            description: None,
            options: Some(vec![]),
            default: None,
        };
        let err = map_to_onboarding_field(&req).unwrap_err();
        assert!(err.to_string().contains("non-empty options"));
    }

    #[test]
    fn map_select_request_no_options_fails() {
        let req = GuestElicitRequest {
            kind: "select".into(),
            key: "network".into(),
            description: None,
            options: None,
            default: None,
        };
        let err = map_to_onboarding_field(&req).unwrap_err();
        assert!(err.to_string().contains("non-empty options"));
    }

    #[test]
    fn map_array_request() {
        let req = GuestElicitRequest {
            kind: "array".into(),
            key: "relays".into(),
            description: Some("Enter relay URLs".into()),
            options: None,
            default: None,
        };
        let field = map_to_onboarding_field(&req).unwrap();
        assert_eq!(field.field_type, OnboardingFieldType::Array);
    }

    #[test]
    fn map_unknown_type_fails() {
        let req = GuestElicitRequest {
            kind: "checkbox".into(),
            key: "foo".into(),
            description: None,
            options: None,
            default: None,
        };
        let err = map_to_onboarding_field(&req).unwrap_err();
        assert!(err.to_string().contains("unknown elicit type"));
    }

    #[test]
    fn map_text_uses_key_as_prompt_when_no_description() {
        let req = GuestElicitRequest {
            kind: "text".into(),
            key: "my_setting".into(),
            description: None,
            options: None,
            default: None,
        };
        let field = map_to_onboarding_field(&req).unwrap();
        assert_eq!(field.prompt, "my_setting");
        assert!(field.description.is_none());
    }

    #[test]
    fn guest_elicit_request_deserializes() {
        let json = r#"{"type":"text","key":"name","description":"Your name"}"#;
        let req: GuestElicitRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.kind, "text");
        assert_eq!(req.key, "name");
        assert_eq!(req.description.as_deref(), Some("Your name"));
        assert!(req.options.is_none());
        assert!(req.default.is_none());
    }

    #[test]
    fn guest_has_secret_request_deserializes() {
        let json = r#"{"key":"api_key"}"#;
        let req: GuestHasSecretRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.key, "api_key");
    }
}
