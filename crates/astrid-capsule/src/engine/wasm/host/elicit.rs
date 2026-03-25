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
        let secret_store = self.secret_store.clone();
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
        let secret_store = self.secret_store.clone();

        secret_store
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
