//! Host function implementation for capsule-level approval requests.
//!
//! Called by WASM guests via the `astrid_request_approval` FFI when a capsule
//! needs human consent for a sensitive action. Checks the shared
//! [`AllowanceStore`] first (instant path), then publishes an
//! [`ApprovalRequired`] IPC event and blocks until the frontend responds.

use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;
use astrid_approval::action::SensitiveAction;
use astrid_approval::{Allowance, AllowanceId, AllowancePattern, AllowanceStore};
use astrid_core::types::Timestamp;
use astrid_crypto::KeyPair;
use astrid_events::AstridEvent;
use astrid_events::ipc::{IpcMessage, IpcPayload};
use extism::{CurrentPlugin, Error, UserData, Val};
use serde::Deserialize;
use uuid::Uuid;

/// Maximum timeout for approval requests (60 seconds).
const MAX_APPROVAL_TIMEOUT_MS: u64 = 60_000;

/// The wire format sent by the SDK's `approval::request` function.
#[derive(Deserialize)]
struct GuestApprovalRequest {
    action: String,
    resource: String,
    risk_level: String,
}

/// Check the allowance store for a matching pattern.
///
/// Builds a `SensitiveAction::ExecuteCommand` from the full resource string
/// so that `CommandPattern` glob matching works against the complete command.
fn check_allowance(store: &AllowanceStore, resource: &str) -> bool {
    let action = SensitiveAction::ExecuteCommand {
        command: resource.to_owned(),
        args: vec![],
    };
    store.find_matching(&action, None).is_some()
}

/// Create a session-scoped allowance from an approval decision.
///
/// For `approve_session`, creates a `CommandPattern` with a subcommand-level
/// glob (e.g. "git push" becomes "git push *"). For `approve_always`, uses
/// the same pattern but with `session_only: false`.
fn create_allowance_from_decision(store: &AllowanceStore, action: &str, decision: &str) {
    let session_only = match decision {
        "approve_session" => true,
        "approve_always" => false,
        _ => return,
    };

    let pattern = AllowancePattern::CommandPattern {
        command: format!("{action} *"),
    };

    // Generate an ephemeral keypair for signing. Session allowances are
    // ephemeral by nature; persistent allowances will get proper runtime
    // key signing when the capability persistence layer is wired.
    let keypair = KeyPair::generate();
    let allowance = Allowance {
        id: AllowanceId::new(),
        action_pattern: pattern,
        created_at: Timestamp::now(),
        expires_at: None,
        max_uses: None,
        uses_remaining: None,
        session_only,
        workspace_root: None,
        signature: keypair.sign(b"capsule-approval"),
    };

    if let Err(e) = store.add_allowance(allowance) {
        tracing::warn!("Failed to add approval allowance: {e}");
    }
}

/// Host function: `astrid_request_approval(request_json) -> response_json`
///
/// Blocks the WASM thread until the frontend user approves or denies, or
/// the request times out. If an allowance already exists, returns immediately.
#[expect(clippy::needless_pass_by_value)]
pub(crate) fn astrid_request_approval_impl(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<HostState>,
) -> Result<(), Error> {
    let request_bytes = util::get_safe_bytes(plugin, &inputs[0], util::MAX_GUEST_PAYLOAD_LEN)?;
    let guest_req: GuestApprovalRequest = serde_json::from_slice(&request_bytes)
        .map_err(|e| Error::msg(format!("invalid approval request JSON: {e}")))?;

    let ud = user_data.get()?;

    // Extract what we need from HostState, then drop the lock before blocking.
    let (allowance_store, event_bus, runtime_handle, capsule_id, cancel_token) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

        let store = state.allowance_store.clone();
        let event_bus = state.event_bus.clone();
        let runtime_handle = state.runtime_handle.clone();
        let capsule_id = state.capsule_id.to_string();
        let cancel_token = state.cancel_token.clone();

        (store, event_bus, runtime_handle, capsule_id, cancel_token)
    };

    // Fast path: check existing allowances.
    if let Some(ref store) = allowance_store
        && check_allowance(store, &guest_req.resource)
    {
        let response = serde_json::to_vec(&serde_json::json!({
            "approved": true,
            "decision": "allowance",
        }))
        .map_err(|e| Error::msg(format!("failed to serialize response: {e}")))?;

        tracing::debug!(
            capsule = %capsule_id,
            action = %guest_req.action,
            resource = %guest_req.resource,
            "Approval auto-granted via existing allowance"
        );

        let mem = plugin.memory_new(&response)?;
        outputs[0] = plugin.memory_to_val(mem);
        return Ok(());
    }

    // Slow path: publish ApprovalRequired and wait for response.
    let request_id = Uuid::new_v4().to_string();
    let response_topic = format!("astrid.v1.approval.response.{request_id}");

    // Subscribe BEFORE publishing to prevent a race.
    let mut receiver = event_bus.subscribe_topic(&response_topic);

    let request_payload = IpcPayload::ApprovalRequired {
        request_id: request_id.clone(),
        action: guest_req.action.clone(),
        resource: guest_req.resource.clone(),
        reason: format!("Capsule '{capsule_id}' requests approval"),
        risk_level: guest_req.risk_level.clone(),
    };
    let message = IpcMessage::new(
        "astrid.v1.approval",
        request_payload,
        Uuid::nil(), // Kernel-originated
    );
    event_bus.publish(AstridEvent::Ipc {
        message,
        metadata: astrid_events::EventMetadata::default(),
    });

    tracing::debug!(
        capsule = %capsule_id,
        action = %guest_req.action,
        resource = %guest_req.resource,
        risk_level = %guest_req.risk_level,
        %request_id,
        "Published approval request, waiting for response"
    );

    // Block until response, timeout, or cancellation.
    let event = runtime_handle.block_on(async {
        tokio::select! {
            result = tokio::time::timeout(
                std::time::Duration::from_millis(MAX_APPROVAL_TIMEOUT_MS),
                receiver.recv(),
            ) => result.ok().flatten(),
            () = cancel_token.cancelled() => None,
        }
    });

    let response_json = match event {
        Some(event) => {
            if let AstridEvent::Ipc { message, .. } = &*event {
                match &message.payload {
                    IpcPayload::ApprovalResponse {
                        decision, reason, ..
                    } => {
                        let approved = matches!(
                            decision.as_str(),
                            "approve" | "approve_session" | "approve_always"
                        );

                        // Create allowance for session/always decisions.
                        if approved && let Some(ref store) = allowance_store {
                            create_allowance_from_decision(store, &guest_req.action, decision);
                        }

                        tracing::info!(
                            capsule = %capsule_id,
                            action = %guest_req.action,
                            %decision,
                            reason = reason.as_deref().unwrap_or("none"),
                            "Approval response received"
                        );

                        serde_json::to_vec(&serde_json::json!({
                            "approved": approved,
                            "decision": decision,
                        }))
                        .map_err(|e| Error::msg(format!("failed to serialize response: {e}")))?
                    },
                    _ => {
                        return Err(Error::msg(
                            "unexpected IPC payload type in approval response",
                        ));
                    },
                }
            } else {
                return Err(Error::msg("unexpected event type in approval response"));
            }
        },
        None => {
            tracing::warn!(
                capsule = %capsule_id,
                action = %guest_req.action,
                "Approval request timed out or was cancelled"
            );
            // Timeout/cancellation = deny
            serde_json::to_vec(&serde_json::json!({
                "approved": false,
                "decision": "deny",
            }))
            .map_err(|e| Error::msg(format!("failed to serialize response: {e}")))?
        },
    };

    let mem = plugin.memory_new(&response_json)?;
    outputs[0] = plugin.memory_to_val(mem);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_approval_request_deserializes() {
        let json = r#"{"action":"git push","resource":"git push origin main","risk_level":"high"}"#;
        let req: GuestApprovalRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.action, "git push");
        assert_eq!(req.resource, "git push origin main");
        assert_eq!(req.risk_level, "high");
    }

    #[test]
    fn check_allowance_matches_command_pattern() {
        let store = AllowanceStore::new();
        let keypair = KeyPair::generate();
        let allowance = Allowance {
            id: AllowanceId::new(),
            action_pattern: AllowancePattern::CommandPattern {
                command: "git push *".into(),
            },
            created_at: Timestamp::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            session_only: true,
            workspace_root: None,
            signature: keypair.sign(b"test"),
        };
        store.add_allowance(allowance).unwrap();

        assert!(check_allowance(&store, "git push origin main"));
        assert!(!check_allowance(&store, "git status"));
    }

    #[test]
    fn check_allowance_returns_false_on_empty_store() {
        let store = AllowanceStore::new();
        assert!(!check_allowance(&store, "git push origin main"));
    }

    #[test]
    fn create_allowance_approve_session() {
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "git push", "approve_session");
        assert_eq!(store.count(), 1);
        // The created pattern should match "git push origin main"
        assert!(check_allowance(&store, "git push origin main"));
    }

    #[test]
    fn create_allowance_approve_always() {
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "docker run", "approve_always");
        assert_eq!(store.count(), 1);
        assert!(check_allowance(&store, "docker run my-image"));
    }

    #[test]
    fn create_allowance_simple_approve_does_nothing() {
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "git push", "approve");
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn create_allowance_deny_does_nothing() {
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "git push", "deny");
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn create_allowance_garbage_decision_does_nothing() {
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "git push", "garbage");
        assert_eq!(store.count(), 0);
        create_allowance_from_decision(&store, "git push", "");
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn check_allowance_with_special_characters() {
        let store = AllowanceStore::new();
        let keypair = KeyPair::generate();
        let allowance = Allowance {
            id: AllowanceId::new(),
            action_pattern: AllowancePattern::CommandPattern {
                command: "git push *".into(),
            },
            created_at: Timestamp::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            session_only: true,
            workspace_root: None,
            signature: keypair.sign(b"test"),
        };
        store.add_allowance(allowance).unwrap();

        // Semicolon-injected command should NOT match "git push *"
        assert!(!check_allowance(&store, "git status; rm -rf /"));
        // Normal match still works
        assert!(check_allowance(&store, "git push --force origin main"));
    }
}
