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

/// Maximum length for action strings from WASM guests.
///
/// Actions longer than this are rejected at the entry point and truncated
/// in the sanitization layer. Prevents DoS via oversized glob pattern
/// compilation.
const MAX_ACTION_LEN: usize = 256;

/// The wire format sent by the SDK's `approval::request` function.
#[derive(Deserialize)]
struct GuestApprovalRequest {
    action: String,
    resource: String,
    risk_level: String,
}

/// Check the allowance store for a matching pattern, consuming limited-use
/// allowances.
///
/// Builds a `SensitiveAction::ExecuteCommand` from the full resource string
/// so that `CommandPattern` glob matching works against the complete command.
/// Uses `find_matching_and_consume` to correctly decrement `uses_remaining`
/// on limited-use allowances.
fn check_allowance(
    store: &AllowanceStore,
    resource: &str,
    workspace_root: Option<&std::path::Path>,
) -> bool {
    let action = SensitiveAction::ExecuteCommand {
        command: resource.to_owned(),
        args: vec![],
    };
    store
        .find_matching_and_consume(&action, workspace_root)
        .is_some()
}

/// Strip control characters from a guest-supplied string in place.
///
/// Used for `resource` and `risk_level` fields that flow into IPC payloads
/// and tracing logs. Prevents ANSI escape sequence injection into
/// terminal-rendered approval prompts.
fn strip_control_chars_inplace(s: &mut String) {
    if s.chars().any(|c| c.is_control()) {
        *s = s.chars().filter(|c| !c.is_control()).collect();
    }
}

/// Sanitize a guest-supplied action string for safe use in glob patterns.
///
/// Defense layer 1: strips control characters and enforces a length cap.
/// Runs BEFORE [`escape_glob_metacharacters`] (layer 2). Together they
/// guarantee that no guest input can produce a dangerous or oversized glob
/// pattern. All printable characters are preserved - shell operators and
/// glob wildcards are handled by downstream layers.
///
/// Logs a warning if control characters were stripped or the string was
/// truncated, identifying the capsule for audit purposes. Does NOT warn
/// for whitespace trimming alone (that is normal, not suspicious).
fn sanitize_action_for_pattern(action: &str, capsule_id: &str) -> String {
    let trimmed = action.trim();
    let sanitized: String = trimmed
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_ACTION_LEN)
        .collect();

    // Only warn for control-char stripping or truncation, not whitespace trim.
    // Compare char counts (not byte lengths) so the log fields correlate with
    // MAX_ACTION_LEN which is a char-count limit.
    let trimmed_chars = trimmed.chars().count();
    let sanitized_chars = sanitized.chars().count();
    if sanitized_chars != trimmed_chars {
        tracing::warn!(
            capsule = %capsule_id,
            original_chars = trimmed_chars,
            sanitized_chars = sanitized_chars,
            "Action string sanitized: control characters stripped or length truncated"
        );
    }

    sanitized
}

/// Escape glob metacharacters in a guest-supplied action string.
///
/// Defense layer 2: escapes glob wildcards (`*`, `?`, `[`, `]`, `{`, `}`,
/// `\`) so they are matched literally. Layer 1
/// ([`sanitize_action_for_pattern`]) strips control characters and enforces
/// length. Layer 3 ([`contains_shell_operators`] in `pattern.rs`) rejects
/// shell injection at match time.
fn escape_glob_metacharacters(action: &str) -> String {
    // Worst case: every char is a glob metacharacter needing a `\` prefix.
    let mut escaped = String::with_capacity(action.len() * 2);
    for c in action.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// Create a session-scoped allowance from an approval decision.
///
/// For `approve_session`, creates a `CommandPattern` with a subcommand-level
/// glob (e.g. "git push" becomes "git push *"). For `approve_always`, uses
/// the same pattern but with `session_only: false`.
fn create_allowance_from_decision(
    store: &AllowanceStore,
    action: &str,
    decision: &str,
    workspace_root: Option<std::path::PathBuf>,
    capsule_id: &str,
) {
    let session_only = match decision {
        "approve_session" => true,
        // FIXME(#382): `approve_always` sets `session_only: false` but the
        // signing key is ephemeral. Treat as session-scoped until the kernel
        // runtime key is threaded through HostState for proper signatures.
        "approve_always" => false,
        // "approve" (once) intentionally creates no allowance. The next
        // identical call will re-prompt. Only "approve_session" and
        // "approve_always" persist across calls.
        _ => return,
    };

    // Layer 1: strip control characters, enforce length cap.
    let sanitized = sanitize_action_for_pattern(action, capsule_id);
    // Empty action after sanitization produces pattern " *" which is
    // meaningless. Skip allowance creation rather than storing a useless entry.
    if sanitized.is_empty() {
        return;
    }
    // Layer 2: escape glob metacharacters so wildcards match literally.
    let sanitized = escape_glob_metacharacters(&sanitized);
    let pattern = AllowancePattern::CommandPattern {
        command: format!("{sanitized} *"),
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
        workspace_root,
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
    let mut guest_req: GuestApprovalRequest = serde_json::from_slice(&request_bytes)
        .map_err(|e| Error::msg(format!("invalid approval request JSON: {e}")))?;

    let ud = user_data.get()?;

    // Extract what we need from HostState, then drop the lock before blocking.
    // Extracted early so capsule_id is available for sanitization logging.
    let (
        allowance_store,
        event_bus,
        runtime_handle,
        capsule_id,
        cancel_token,
        host_semaphore,
        workspace_root,
    ) = {
        let state = ud
            .lock()
            .map_err(|e| Error::msg(format!("host state lock poisoned: {e}")))?;

        let store = state.allowance_store.clone();
        let event_bus = state.event_bus.clone();
        let runtime_handle = state.runtime_handle.clone();
        let capsule_id = state.capsule_id.to_string();
        let cancel_token = state.cancel_token.clone();
        let host_semaphore = state.host_semaphore.clone();
        let workspace = state.workspace_root.clone();

        (
            store,
            event_bus,
            runtime_handle,
            capsule_id,
            cancel_token,
            host_semaphore,
            workspace,
        )
    };

    // Validate and sanitize all guest-supplied strings at the entry point.
    // This ensures IPC payloads and log messages contain clean values.
    let action_char_count = guest_req.action.chars().count();
    if action_char_count > MAX_ACTION_LEN {
        return Err(Error::msg(format!(
            "approval request action exceeds maximum length ({action_char_count} > {MAX_ACTION_LEN})",
        )));
    }
    // Single source of truth: sanitize_action_for_pattern strips control
    // chars, trims whitespace, and enforces length. Applied here so the
    // cleaned value flows through to IPC payloads and logs.
    guest_req.action = sanitize_action_for_pattern(&guest_req.action, &capsule_id);
    // Strip control characters from resource and risk_level too - they are
    // guest-controlled and flow into IPC payloads and tracing logs. A
    // malicious capsule could embed ANSI escape sequences to spoof
    // terminal-rendered approval prompts.
    strip_control_chars_inplace(&mut guest_req.resource);
    strip_control_chars_inplace(&mut guest_req.risk_level);

    let ws_path = Some(workspace_root.as_path());

    // Fast path: check existing allowances.
    if let Some(ref store) = allowance_store
        && check_allowance(store, &guest_req.resource, ws_path)
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

    // Block until response, timeout, or cancellation. Routed through the host
    // semaphore to bound concurrent blocking operations across all capsules.
    let event = util::bounded_block_on_cancellable(
        &runtime_handle,
        &host_semaphore,
        &cancel_token,
        async {
            tokio::time::timeout(
                std::time::Duration::from_millis(MAX_APPROVAL_TIMEOUT_MS),
                receiver.recv(),
            )
            .await
            .ok()
            .flatten()
        },
    )
    .flatten();

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
                            create_allowance_from_decision(
                                store,
                                &guest_req.action,
                                decision,
                                Some(workspace_root.clone()),
                                &capsule_id,
                            );
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

        assert!(check_allowance(&store, "git push origin main", None));
        assert!(!check_allowance(&store, "git status", None));
    }

    #[test]
    fn check_allowance_returns_false_on_empty_store() {
        let store = AllowanceStore::new();
        assert!(!check_allowance(&store, "git push origin main", None));
    }

    #[test]
    fn create_allowance_approve_session() {
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "git push", "approve_session", None, "test");
        assert_eq!(store.count(), 1);
        // The created pattern should match "git push origin main"
        assert!(check_allowance(&store, "git push origin main", None));
    }

    #[test]
    fn create_allowance_approve_always() {
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "docker run", "approve_always", None, "test");
        assert_eq!(store.count(), 1);
        assert!(check_allowance(&store, "docker run my-image", None));
    }

    #[test]
    fn create_allowance_simple_approve_does_nothing() {
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "git push", "approve", None, "test");
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn create_allowance_deny_does_nothing() {
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "git push", "deny", None, "test");
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn create_allowance_garbage_decision_does_nothing() {
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "git push", "garbage", None, "test");
        assert_eq!(store.count(), 0);
        create_allowance_from_decision(&store, "git push", "", None, "test");
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
        assert!(!check_allowance(&store, "git status; rm -rf /", None));
        // Normal match still works
        assert!(check_allowance(
            &store,
            "git push --force origin main",
            None
        ));
    }

    #[test]
    fn escape_glob_metacharacters_preserves_normal_chars() {
        assert_eq!(escape_glob_metacharacters("git push"), "git push");
        assert_eq!(
            escape_glob_metacharacters("npm install @types/react"),
            "npm install @types/react"
        );
        assert_eq!(escape_glob_metacharacters("my-tool_v2.0"), "my-tool_v2.0");
    }

    #[test]
    fn escape_glob_metacharacters_escapes_wildcards() {
        assert_eq!(escape_glob_metacharacters("*"), "\\*");
        assert_eq!(escape_glob_metacharacters("git *"), "git \\*");
        assert_eq!(escape_glob_metacharacters("git[status]"), "git\\[status\\]");
        assert_eq!(escape_glob_metacharacters("cmd?"), "cmd\\?");
    }

    #[test]
    fn create_allowance_with_wildcard_in_action_is_not_overly_broad() {
        let store = AllowanceStore::new();
        // A malicious capsule sends action = "*" hoping to get pattern "* *"
        // After escaping, pattern becomes "\* *" which won't match normal commands.
        create_allowance_from_decision(&store, "*", "approve_session", None, "test");
        assert_eq!(store.count(), 1);
        assert!(!check_allowance(&store, "git push origin main", None));
    }

    #[test]
    fn create_allowance_empty_action() {
        // Empty action after sanitization produces no allowance - the pattern
        // would be " *" which is meaningless.
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "", "approve_session", None, "test");
        assert_eq!(store.count(), 0);
        assert!(!check_allowance(&store, "git push", None));
    }

    #[test]
    fn approve_once_does_not_create_allowance() {
        // "approve" (one-time) should NOT create an allowance. The next
        // identical call will re-prompt the user.
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "git push", "approve", None, "test");
        assert_eq!(store.count(), 0);
        assert!(!check_allowance(&store, "git push origin main", None));
    }

    // --- sanitize_action_for_pattern tests ---

    #[test]
    fn sanitize_action_preserves_shell_fragments() {
        // Legitimate commands with shell-like characters must pass through
        // unchanged - they are handled by escape (layer 2) and
        // contains_shell_operators (layer 3), not this layer.
        assert_eq!(
            sanitize_action_for_pattern("python -c 'print(\"hello\")'", "test"),
            "python -c 'print(\"hello\")'"
        );
        assert_eq!(
            sanitize_action_for_pattern("awk '{print $1}' file.txt", "test"),
            "awk '{print $1}' file.txt"
        );
        assert_eq!(
            sanitize_action_for_pattern("bash -c 'echo $HOME'", "test"),
            "bash -c 'echo $HOME'"
        );
        assert_eq!(
            sanitize_action_for_pattern("g++ main.cpp", "test"),
            "g++ main.cpp"
        );
        assert_eq!(
            sanitize_action_for_pattern("npm install @types/react", "test"),
            "npm install @types/react"
        );
        assert_eq!(
            sanitize_action_for_pattern("docker run ubuntu:latest", "test"),
            "docker run ubuntu:latest"
        );
    }

    #[test]
    fn sanitize_action_preserves_glob_chars_for_escaping() {
        // Glob metacharacters are printable and pass through this layer.
        // They are neutralized by escape_glob_metacharacters (layer 2).
        assert_eq!(sanitize_action_for_pattern("*", "test"), "*");
        assert_eq!(sanitize_action_for_pattern("git *", "test"), "git *");
        assert_eq!(sanitize_action_for_pattern("cmd?", "test"), "cmd?");
        assert_eq!(
            sanitize_action_for_pattern("git[status]", "test"),
            "git[status]"
        );
    }

    #[test]
    fn sanitize_action_strips_control_characters() {
        assert_eq!(sanitize_action_for_pattern("git\0push", "test"), "gitpush");
        assert_eq!(sanitize_action_for_pattern("git\rpush", "test"), "gitpush");
        assert_eq!(
            sanitize_action_for_pattern("git\x1b[31mpush", "test"),
            "git[31mpush"
        );
        assert_eq!(sanitize_action_for_pattern("git\tpush", "test"), "gitpush");
        assert_eq!(sanitize_action_for_pattern("git\npush", "test"), "gitpush");
    }

    #[test]
    fn sanitize_action_truncates_long_strings() {
        let long_action = "a".repeat(500);
        let sanitized = sanitize_action_for_pattern(&long_action, "test");
        assert_eq!(sanitized.chars().count(), MAX_ACTION_LEN);
    }

    #[test]
    fn sanitize_action_exact_limit_no_change() {
        // Exactly MAX_ACTION_LEN printable chars should pass through unchanged.
        let action = "a".repeat(MAX_ACTION_LEN);
        let sanitized = sanitize_action_for_pattern(&action, "test");
        assert_eq!(sanitized, action);
        assert_eq!(sanitized.chars().count(), MAX_ACTION_LEN);
    }

    #[test]
    fn sanitize_action_truncates_multibyte_chars() {
        // 200 ASCII + 100 x U+0100 ("Ā", 2 bytes each) = 300 chars.
        // Truncation should produce exactly 256 chars.
        let action = "a".repeat(200) + &"\u{0100}".repeat(100);
        assert_eq!(action.chars().count(), 300);
        let sanitized = sanitize_action_for_pattern(&action, "test");
        assert_eq!(sanitized.chars().count(), MAX_ACTION_LEN);
        assert!(sanitized.starts_with(&"a".repeat(200)));
    }

    #[test]
    fn sanitize_action_trims_whitespace() {
        assert_eq!(
            sanitize_action_for_pattern("  git push  ", "test"),
            "git push"
        );
    }

    #[test]
    fn create_allowance_whitespace_padded_action() {
        // Whitespace-padded action should flow through both layers and
        // produce a working session allowance.
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "  git push  ", "approve_session", None, "test");
        assert_eq!(store.count(), 1);
        // After trim: "git push", pattern: "git push *"
        assert!(check_allowance(&store, "git push origin main", None));
        assert!(!check_allowance(&store, "git status", None));
    }

    #[test]
    fn create_allowance_combined_attack() {
        // A malicious capsule sends action with control chars + glob wildcards.
        // Layer 1 strips control chars, layer 2 escapes glob chars.
        // The resulting pattern must NOT match unintended commands.
        let store = AllowanceStore::new();
        let attack = "git\0 *\x1b[31m";
        create_allowance_from_decision(&store, attack, "approve_session", None, "test");
        assert_eq!(store.count(), 1);
        // After sanitization: "git *[31m" (control chars stripped)
        // After escaping: "git \*\[31m" (glob chars escaped)
        // Pattern: "git \*\[31m *"
        // This should NOT match normal git commands.
        assert!(!check_allowance(&store, "git push origin main", None));
        assert!(!check_allowance(&store, "git status", None));
    }

    #[test]
    fn create_allowance_null_byte_attack() {
        // Null bytes stripped, pattern still safe.
        let store = AllowanceStore::new();
        create_allowance_from_decision(&store, "git\0push", "approve_session", None, "test");
        assert_eq!(store.count(), 1);
        // After sanitization: "gitpush", pattern: "gitpush *"
        // Does not match "git push" (different string).
        assert!(!check_allowance(&store, "git push origin main", None));
        // Matches only literal "gitpush ..." which is not a real command.
        assert!(check_allowance(&store, "gitpush something", None));
    }
}
