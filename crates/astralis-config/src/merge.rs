//! Deep merge of TOML values with restriction enforcement.
//!
//! The merge operates on raw [`toml::Value`] trees rather than deserialized
//! structs. This correctly handles "absent vs default" — a missing key in a
//! TOML table will not override the base layer.

use std::collections::HashMap;

use tracing::warn;

/// Which configuration layer a value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigLayer {
    /// Compiled-in defaults (`defaults.toml`).
    Defaults,
    /// System-wide configuration (`/etc/astralis/config.toml`).
    System,
    /// User-level configuration (`~/.astralis/config.toml`).
    User,
    /// Workspace-level configuration (`{workspace}/.astralis/config.toml`).
    Workspace,
    /// Environment variable fallback.
    Environment,
}

impl std::fmt::Display for ConfigLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Defaults => write!(f, "defaults"),
            Self::System => write!(f, "system (/etc/astralis/config.toml)"),
            Self::User => write!(f, "user (~/.astralis/config.toml)"),
            Self::Workspace => write!(f, "workspace (.astralis/config.toml)"),
            Self::Environment => write!(f, "environment variable"),
        }
    }
}

/// Tracks which layer set each field's value.
pub type FieldSources = HashMap<String, ConfigLayer>;

/// Recursively deep-merge `overlay` into `base`.
///
/// - Tables merge recursively per-field.
/// - Scalars and arrays from the overlay **replace** the base value.
pub fn deep_merge(base: &mut toml::Value, overlay: &toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, overlay_val) in overlay_table {
                if let Some(base_val) = base_table.get_mut(key) {
                    deep_merge(base_val, overlay_val);
                } else {
                    base_table.insert(key.clone(), overlay_val.clone());
                }
            }
        },
        (base, overlay) => {
            *base = overlay.clone();
        },
    }
}

/// Deep-merge `overlay` into `base`, recording which layer set each leaf
/// field. `prefix` is the dotted path prefix (e.g. `"model"`) and `layer`
/// identifies where the overlay came from.
pub fn deep_merge_tracking(
    base: &mut toml::Value,
    overlay: &toml::Value,
    prefix: &str,
    layer: &ConfigLayer,
    sources: &mut FieldSources,
) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, overlay_val) in overlay_table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };

                if let Some(base_val) = base_table.get_mut(key) {
                    if overlay_val.is_table() {
                        deep_merge_tracking(base_val, overlay_val, &path, layer, sources);
                    } else {
                        *base_val = overlay_val.clone();
                        sources.insert(path, layer.clone());
                    }
                } else {
                    base_table.insert(key.clone(), overlay_val.clone());
                    record_all_leaves(overlay_val, &path, layer, sources);
                }
            }
        },
        (base, overlay) => {
            *base = overlay.clone();
            sources.insert(prefix.to_owned(), layer.clone());
        },
    }
}

/// Walk a value tree and record all leaf paths with their source layer.
fn record_all_leaves(
    val: &toml::Value,
    prefix: &str,
    layer: &ConfigLayer,
    sources: &mut FieldSources,
) {
    if let toml::Value::Table(table) = val {
        for (key, child) in table {
            let path = format!("{prefix}.{key}");
            record_all_leaves(child, &path, layer, sources);
        }
    } else {
        sources.insert(prefix.to_owned(), layer.clone());
    }
}

/// Enforce that the workspace layer can only **tighten** security, not loosen
/// it. Call this after merging the workspace layer but before final
/// deserialization.
///
/// `baseline` is the merged config *before* the workspace layer was applied.
/// This ensures enforcement works even when no user config file exists —
/// the defaults serve as the baseline.
#[allow(clippy::too_many_lines)]
pub fn enforce_restrictions(
    merged: &mut toml::Value,
    baseline: &toml::Value,
    workspace_layer: &toml::Value,
) {
    // Budget: can only decrease.
    clamp_max(
        merged,
        baseline,
        workspace_layer,
        &["budget", "session_max_usd"],
        "budget.session_max_usd",
    );
    clamp_max(
        merged,
        baseline,
        workspace_layer,
        &["budget", "per_action_max_usd"],
        "budget.per_action_max_usd",
    );

    // Max argument size: can only decrease.
    clamp_max_int(
        merged,
        baseline,
        workspace_layer,
        &["security", "policy", "max_argument_size"],
        "security.policy.max_argument_size",
    );

    // Booleans that can only become true (workspace cannot disable).
    enforce_bool_only_true(
        merged,
        workspace_layer,
        &["security", "policy", "require_approval_for_delete"],
        "security.policy.require_approval_for_delete",
    );
    enforce_bool_only_true(
        merged,
        workspace_layer,
        &["security", "policy", "require_approval_for_network"],
        "security.policy.require_approval_for_network",
    );

    // Union array fields: workspace can only add, not remove.
    union_string_arrays(
        merged,
        baseline,
        workspace_layer,
        &["security", "policy", "blocked_tools"],
        "security.policy.blocked_tools",
    );
    union_string_arrays(
        merged,
        baseline,
        workspace_layer,
        &["security", "policy", "denied_paths"],
        "security.policy.denied_paths",
    );
    union_string_arrays(
        merged,
        baseline,
        workspace_layer,
        &["security", "policy", "denied_hosts"],
        "security.policy.denied_hosts",
    );

    // --- Step 3: Additional restriction enforcement ---

    // Workspace mode: can only tighten (safe < guided < autonomous).
    enforce_mode_tighten(
        merged,
        baseline,
        workspace_layer,
        &["workspace", "mode"],
        "workspace.mode",
        &["safe", "guided", "autonomous"],
    );

    // Escape policy: can only tighten (deny < ask < allow).
    enforce_mode_tighten(
        merged,
        baseline,
        workspace_layer,
        &["workspace", "escape_policy"],
        "workspace.escape_policy",
        &["deny", "ask", "allow"],
    );

    // workspace.never_allow: union (can only add).
    union_string_arrays(
        merged,
        baseline,
        workspace_layer,
        &["workspace", "never_allow"],
        "workspace.never_allow",
    );

    // security.require_signatures: can only become true.
    enforce_bool_only_true(
        merged,
        workspace_layer,
        &["security", "require_signatures"],
        "security.require_signatures",
    );

    // security.approval_timeout_secs: can only decrease.
    clamp_max_int(
        merged,
        baseline,
        workspace_layer,
        &["security", "approval_timeout_secs"],
        "security.approval_timeout_secs",
    );

    // security.policy.approval_required_tools: union (can only add).
    union_string_arrays(
        merged,
        baseline,
        workspace_layer,
        &["security", "policy", "approval_required_tools"],
        "security.policy.approval_required_tools",
    );

    // security.policy.allowed_paths: cannot expand beyond baseline.
    block_workspace_expansion(
        merged,
        baseline,
        workspace_layer,
        &["security", "policy", "allowed_paths"],
        "security.policy.allowed_paths",
    );

    // security.policy.allowed_hosts: cannot expand beyond baseline.
    block_workspace_expansion(
        merged,
        baseline,
        workspace_layer,
        &["security", "policy", "allowed_hosts"],
        "security.policy.allowed_hosts",
    );

    // workspace.auto_allow_read: cannot expand beyond baseline.
    block_workspace_expansion(
        merged,
        baseline,
        workspace_layer,
        &["workspace", "auto_allow_read"],
        "workspace.auto_allow_read",
    );

    // workspace.auto_allow_write: cannot expand beyond baseline.
    block_workspace_expansion(
        merged,
        baseline,
        workspace_layer,
        &["workspace", "auto_allow_write"],
        "workspace.auto_allow_write",
    );

    // model.api_key: workspace cannot override.
    block_workspace_override(
        merged,
        baseline,
        workspace_layer,
        &["model", "api_key"],
        "model.api_key",
    );

    // model.api_url: workspace cannot override.
    block_workspace_override(
        merged,
        baseline,
        workspace_layer,
        &["model", "api_url"],
        "model.api_url",
    );

    // hooks.allow_wasm_hooks: cannot enable (only disable).
    enforce_bool_only_false(
        merged,
        workspace_layer,
        &["hooks", "allow_wasm_hooks"],
        "hooks.allow_wasm_hooks",
    );

    // hooks.allow_agent_hooks: cannot enable (only disable).
    enforce_bool_only_false(
        merged,
        workspace_layer,
        &["hooks", "allow_agent_hooks"],
        "hooks.allow_agent_hooks",
    );

    // rate_limits: can only decrease.
    clamp_max_int(
        merged,
        baseline,
        workspace_layer,
        &["rate_limits", "elicitation_per_server_per_min"],
        "rate_limits.elicitation_per_server_per_min",
    );
    clamp_max_int(
        merged,
        baseline,
        workspace_layer,
        &["rate_limits", "max_pending_requests"],
        "rate_limits.max_pending_requests",
    );

    // budget.warn_at_percent: can only decrease.
    clamp_max_int(
        merged,
        baseline,
        workspace_layer,
        &["budget", "warn_at_percent"],
        "budget.warn_at_percent",
    );

    // subagents: limits can only decrease from workspace.
    clamp_max_int(
        merged,
        baseline,
        workspace_layer,
        &["subagents", "max_concurrent"],
        "subagents.max_concurrent",
    );
    clamp_max_int(
        merged,
        baseline,
        workspace_layer,
        &["subagents", "max_depth"],
        "subagents.max_depth",
    );
    clamp_max_int(
        merged,
        baseline,
        workspace_layer,
        &["subagents", "timeout_secs"],
        "subagents.timeout_secs",
    );

    // retry: limits can only decrease from workspace.
    clamp_max_int(
        merged,
        baseline,
        workspace_layer,
        &["retry", "llm_max_attempts"],
        "retry.llm_max_attempts",
    );
    clamp_max_int(
        merged,
        baseline,
        workspace_layer,
        &["retry", "mcp_max_attempts"],
        "retry.mcp_max_attempts",
    );

    // timeouts.approval_secs: can only decrease.
    clamp_max_int(
        merged,
        baseline,
        workspace_layer,
        &["timeouts", "approval_secs"],
        "timeouts.approval_secs",
    );

    // timeouts.idle_secs: can only decrease (prevent workspace keeping
    // sessions alive indefinitely).
    clamp_max_int(
        merged,
        baseline,
        workspace_layer,
        &["timeouts", "idle_secs"],
        "timeouts.idle_secs",
    );

    // hooks.allow_http_hooks: cannot enable (only disable).
    enforce_bool_only_false(
        merged,
        workspace_layer,
        &["hooks", "allow_http_hooks"],
        "hooks.allow_http_hooks",
    );

    // hooks.allow_command_hooks: cannot enable (only disable).
    enforce_bool_only_false(
        merged,
        workspace_layer,
        &["hooks", "allow_command_hooks"],
        "hooks.allow_command_hooks",
    );

    // --- Step 4: Prevent workspace server injection ---
    sanitize_workspace_servers(merged, baseline, workspace_layer);
}

/// Clamp a float field so workspace cannot increase it beyond baseline.
fn clamp_max(
    merged: &mut toml::Value,
    baseline: &toml::Value,
    workspace: &toml::Value,
    path: &[&str],
    field_name: &str,
) {
    let baseline_val = get_nested(baseline, path).and_then(toml::Value::as_float);
    let ws_val = get_nested(workspace, path).and_then(toml::Value::as_float);

    if let (Some(base_v), Some(ws_v)) = (baseline_val, ws_val)
        && ws_v > base_v
    {
        warn!(
            "Workspace config tried to increase {field_name} from {base_v} to {ws_v}; \
             clamping to {base_v}"
        );
        set_nested(merged, path, toml::Value::Float(base_v));
    }
}

/// Clamp an integer field so workspace cannot increase it beyond baseline.
fn clamp_max_int(
    merged: &mut toml::Value,
    baseline: &toml::Value,
    workspace: &toml::Value,
    path: &[&str],
    field_name: &str,
) {
    let baseline_val = get_nested(baseline, path).and_then(toml::Value::as_integer);
    let ws_val = get_nested(workspace, path).and_then(toml::Value::as_integer);

    if let (Some(base_v), Some(ws_v)) = (baseline_val, ws_val)
        && ws_v > base_v
    {
        warn!(
            "Workspace config tried to increase {field_name} from {base_v} to {ws_v}; \
             clamping to {base_v}"
        );
        set_nested(merged, path, toml::Value::Integer(base_v));
    }
}

/// Ensure a boolean field can only become `true`, never go from `true` to
/// `false`.
fn enforce_bool_only_true(
    merged: &mut toml::Value,
    workspace: &toml::Value,
    path: &[&str],
    field_name: &str,
) {
    if let Some(ws_val) = get_nested(workspace, path).and_then(toml::Value::as_bool)
        && !ws_val
    {
        warn!(
            "Workspace config tried to disable {field_name}; \
             forcing to true (workspace can only enable, not disable)"
        );
        set_nested(merged, path, toml::Value::Boolean(true));
    }
}

/// Ensure a boolean field can only become `false`, never go from `false` to
/// `true`. Used for dangerous capabilities that must not be workspace-enabled.
fn enforce_bool_only_false(
    merged: &mut toml::Value,
    workspace: &toml::Value,
    path: &[&str],
    field_name: &str,
) {
    if let Some(ws_val) = get_nested(workspace, path).and_then(toml::Value::as_bool)
        && ws_val
    {
        warn!(
            "Workspace config tried to enable {field_name}; \
             forcing to false (workspace cannot enable this)"
        );
        set_nested(merged, path, toml::Value::Boolean(false));
    }
}

/// Union the workspace array with the baseline array: workspace can only add
/// entries, not remove them. The result is the set union.
fn union_string_arrays(
    merged: &mut toml::Value,
    baseline: &toml::Value,
    workspace: &toml::Value,
    path: &[&str],
    field_name: &str,
) {
    let baseline_arr = get_nested(baseline, path).and_then(|v| v.as_array().cloned());
    let ws_arr = get_nested(workspace, path).and_then(|v| v.as_array().cloned());

    if let (Some(baseline_items), Some(_ws_items)) = (baseline_arr, ws_arr) {
        // Compute the union: start with what's in merged (which includes ws
        // overlay), then ensure all baseline items are present.
        let merged_arr = get_nested(merged, path)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();

        let mut result = merged_arr;
        for item in &baseline_items {
            if !result.contains(item) {
                warn!(
                    "Workspace config removed an entry from {field_name}; restoring it \
                     (workspace can only add, not remove)"
                );
                result.push(item.clone());
            }
        }

        set_nested(merged, path, toml::Value::Array(result));
    }
}

/// Enforce that a string field representing an ordered mode can only become
/// stricter. Modes are ordered from strictest (index 0) to most permissive.
fn enforce_mode_tighten(
    merged: &mut toml::Value,
    baseline: &toml::Value,
    workspace: &toml::Value,
    path: &[&str],
    field_name: &str,
    ordered_modes: &[&str],
) {
    let baseline_str = get_nested(baseline, path).and_then(toml::Value::as_str);
    let ws_str = get_nested(workspace, path).and_then(toml::Value::as_str);

    if let (Some(base_s), Some(ws_s)) = (baseline_str, ws_str) {
        let base_idx = ordered_modes.iter().position(|m| *m == base_s);
        let ws_idx = ordered_modes.iter().position(|m| *m == ws_s);

        if let (Some(b_idx), Some(w_idx)) = (base_idx, ws_idx)
            && w_idx > b_idx
        {
            warn!(
                "Workspace config tried to escalate {field_name} from \"{base_s}\" to \
                 \"{ws_s}\"; reverting to \"{base_s}\""
            );
            set_nested(merged, path, toml::Value::String(base_s.to_owned()));
        }
    }
}

/// Block workspace from overriding a field entirely. If the workspace sets
/// this field, revert to the baseline value.
fn block_workspace_override(
    merged: &mut toml::Value,
    baseline: &toml::Value,
    workspace: &toml::Value,
    path: &[&str],
    field_name: &str,
) {
    if get_nested(workspace, path).is_some() {
        warn!(
            "Workspace config tried to override {field_name}; \
             reverting to baseline (workspace cannot set this field)"
        );
        if let Some(base_val) = get_nested(baseline, path) {
            set_nested(merged, path, base_val.clone());
        } else {
            // Baseline didn't have it, remove from merged.
            remove_nested(merged, path);
        }
    }
}

/// Block workspace from expanding an array beyond what the baseline allows.
/// If baseline is empty, workspace cannot add entries. If baseline has entries,
/// the workspace result must be a subset.
fn block_workspace_expansion(
    merged: &mut toml::Value,
    baseline: &toml::Value,
    workspace: &toml::Value,
    path: &[&str],
    field_name: &str,
) {
    let ws_arr = get_nested(workspace, path).and_then(|v| v.as_array().cloned());

    if let Some(ws_items) = ws_arr {
        let baseline_arr = get_nested(baseline, path)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();

        // Check if workspace added items not in baseline.
        let mut had_expansion = false;
        for item in &ws_items {
            if !baseline_arr.contains(item) {
                had_expansion = true;
                break;
            }
        }

        if had_expansion {
            warn!(
                "Workspace config tried to expand {field_name} beyond baseline; \
                 reverting to baseline"
            );
            set_nested(merged, path, toml::Value::Array(baseline_arr));
        }
    }
}

/// Security-critical fields that a workspace must not change on baseline servers.
const PROTECTED_SERVER_FIELDS: &[&str] =
    &["command", "args", "env", "cwd", "binary_hash", "trusted"];

/// Prevent workspace-injected servers from being trusted or auto-started,
/// and protect security-critical fields on baseline servers.
fn sanitize_workspace_servers(
    merged: &mut toml::Value,
    baseline: &toml::Value,
    workspace: &toml::Value,
) {
    let ws_servers = get_nested(workspace, &["servers"])
        .and_then(toml::Value::as_table)
        .cloned();

    let Some(ws_servers) = ws_servers else {
        return;
    };

    let baseline_servers = get_nested(baseline, &["servers"])
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default();

    for server_name in ws_servers.keys() {
        if let Some(baseline_server) = baseline_servers.get(server_name) {
            // Server exists in baseline — protect security-critical fields.
            // If the workspace tries to change command/args/env/cwd/binary_hash/trusted,
            // revert those fields to the baseline values.
            let baseline_table = baseline_server.as_table();
            for &field in PROTECTED_SERVER_FIELDS {
                let field_path = &["servers", server_name.as_str(), field];
                let ws_field_val = get_nested(workspace, &["servers", server_name.as_str(), field]);
                let baseline_field_val = baseline_table.and_then(|t| t.get(field));

                if ws_field_val.is_some() {
                    // Workspace is trying to set this protected field.
                    if let Some(base_val) = baseline_field_val {
                        if ws_field_val != Some(base_val) {
                            warn!(
                                "Workspace tried to change protected field '{field}' on \
                                 baseline server '{server_name}'; reverting to baseline value"
                            );
                            set_nested(merged, field_path, base_val.clone());
                        }
                    } else {
                        // Baseline didn't have this field but workspace is trying to add it.
                        warn!(
                            "Workspace tried to add protected field '{field}' to \
                             baseline server '{server_name}'; removing"
                        );
                        remove_nested(merged, field_path);
                    }
                }
            }
            continue;
        }

        // Workspace-injected server: force untrusted and no auto-start.
        let trusted_path = &["servers", server_name.as_str(), "trusted"];
        let auto_start_path = &["servers", server_name.as_str(), "auto_start"];

        if get_nested(merged, trusted_path)
            .and_then(toml::Value::as_bool)
            .unwrap_or(false)
        {
            warn!("Workspace-injected server '{server_name}' had trusted=true; forcing to false");
            set_nested(merged, trusted_path, toml::Value::Boolean(false));
        }

        if get_nested(merged, auto_start_path)
            .and_then(toml::Value::as_bool)
            .unwrap_or(false)
        {
            warn!(
                "Workspace-injected server '{server_name}' had auto_start=true; forcing to false"
            );
            set_nested(merged, auto_start_path, toml::Value::Boolean(false));
        }
    }
}

/// Remove a value at a nested path.
fn remove_nested(val: &mut toml::Value, path: &[&str]) {
    if path.is_empty() {
        return;
    }

    let mut current = val;
    for segment in &path[..path.len() - 1] {
        let Some(next) = current.as_table_mut().and_then(|t| t.get_mut(*segment)) else {
            return;
        };
        current = next;
    }

    if let Some(table) = current.as_table_mut() {
        table.remove(path[path.len() - 1]);
    }
}

/// Navigate into a nested `toml::Value` by dotted path segments.
fn get_nested<'a>(val: &'a toml::Value, path: &[&str]) -> Option<&'a toml::Value> {
    let mut current = val;
    for segment in path {
        current = current.as_table()?.get(*segment)?;
    }
    Some(current)
}

/// Set a value at a nested path, creating intermediate tables as needed.
fn set_nested(val: &mut toml::Value, path: &[&str], new_val: toml::Value) {
    if path.is_empty() {
        return;
    }

    let mut current = val;
    for segment in &path[..path.len() - 1] {
        let Some(next) = current.as_table_mut().and_then(|t| t.get_mut(*segment)) else {
            warn!("set_nested: missing intermediate table at '{segment}'; skipping");
            return;
        };
        current = next;
    }

    if let Some(table) = current.as_table_mut() {
        table.insert(path[path.len() - 1].to_owned(), new_val);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deep_merge_scalars() {
        let mut base: toml::Value = toml::from_str(
            r#"
            [model]
            provider = "claude"
            max_tokens = 4096
        "#,
        )
        .unwrap();

        let overlay: toml::Value = toml::from_str(
            r#"
            [model]
            max_tokens = 8192
        "#,
        )
        .unwrap();

        deep_merge(&mut base, &overlay);

        let table = base.as_table().unwrap();
        let model = table["model"].as_table().unwrap();
        assert_eq!(model["provider"].as_str().unwrap(), "claude");
        assert_eq!(model["max_tokens"].as_integer().unwrap(), 8192);
    }

    #[test]
    fn test_deep_merge_new_keys() {
        let mut base: toml::Value = toml::from_str(
            r#"
            [model]
            provider = "claude"
        "#,
        )
        .unwrap();

        let overlay: toml::Value = toml::from_str(
            r#"
            [model]
            api_key = "sk-test"
            [budget]
            session_max_usd = 50.0
        "#,
        )
        .unwrap();

        deep_merge(&mut base, &overlay);

        let table = base.as_table().unwrap();
        let model = table["model"].as_table().unwrap();
        assert_eq!(model["api_key"].as_str().unwrap(), "sk-test");
        assert!(table.contains_key("budget"));
    }

    #[test]
    fn test_deep_merge_tracking() {
        let mut base: toml::Value = toml::from_str(
            r#"
            [model]
            provider = "claude"
            max_tokens = 4096
        "#,
        )
        .unwrap();

        let overlay: toml::Value = toml::from_str(
            r#"
            [model]
            max_tokens = 8192
        "#,
        )
        .unwrap();

        let mut sources = FieldSources::new();
        deep_merge_tracking(&mut base, &overlay, "", &ConfigLayer::User, &mut sources);

        assert_eq!(sources.get("model.max_tokens"), Some(&ConfigLayer::User));
        assert!(!sources.contains_key("model.provider"));
    }

    // ---- Original restriction tests ----

    #[test]
    fn test_enforce_restrictions_budget_clamp() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [budget]
            session_max_usd = 100.0
            per_action_max_usd = 10.0
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [budget]
            session_max_usd = 200.0
            per_action_max_usd = 5.0
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let budget = merged["budget"].as_table().unwrap();
        assert_eq!(budget["session_max_usd"].as_float().unwrap(), 100.0);
        assert_eq!(budget["per_action_max_usd"].as_float().unwrap(), 5.0);
    }

    #[test]
    fn test_enforce_restrictions_bool_only_true() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [security.policy]
            require_approval_for_delete = true
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [security.policy]
            require_approval_for_delete = false
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let policy = merged["security"]["policy"].as_table().unwrap();
        assert!(policy["require_approval_for_delete"].as_bool().unwrap());
    }

    // ---- Step 2: Restrictions work without user config ----

    #[test]
    fn test_restrictions_work_without_user_config() {
        // Baseline includes defaults (no user file).
        let baseline: toml::Value = toml::from_str(
            r#"
            [budget]
            session_max_usd = 100.0
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [budget]
            session_max_usd = 999.0
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert_eq!(
            merged["budget"]["session_max_usd"].as_float().unwrap(),
            100.0
        );
    }

    #[test]
    fn test_blocked_tools_union_works_without_user_config() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [security.policy]
            blocked_tools = ["sudo", "rm -rf /"]
        "#,
        )
        .unwrap();

        // Workspace tries to remove "sudo".
        let workspace: toml::Value = toml::from_str(
            r#"
            [security.policy]
            blocked_tools = ["rm -rf /"]
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let blocked = merged["security"]["policy"]["blocked_tools"]
            .as_array()
            .unwrap();
        let blocked_strs: Vec<&str> = blocked.iter().filter_map(|v| v.as_str()).collect();
        assert!(blocked_strs.contains(&"sudo"));
        assert!(blocked_strs.contains(&"rm -rf /"));
    }

    // ---- Step 3: Mode tighten ----

    #[test]
    fn test_workspace_mode_cannot_escalate() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [workspace]
            mode = "safe"
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [workspace]
            mode = "autonomous"
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert_eq!(merged["workspace"]["mode"].as_str().unwrap(), "safe");
    }

    #[test]
    fn test_workspace_mode_can_tighten() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [workspace]
            mode = "autonomous"
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [workspace]
            mode = "safe"
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert_eq!(merged["workspace"]["mode"].as_str().unwrap(), "safe");
    }

    #[test]
    fn test_escape_policy_cannot_escalate() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [workspace]
            escape_policy = "ask"
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [workspace]
            escape_policy = "allow"
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert_eq!(
            merged["workspace"]["escape_policy"].as_str().unwrap(),
            "ask"
        );
    }

    #[test]
    fn test_escape_policy_can_tighten() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [workspace]
            escape_policy = "allow"
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [workspace]
            escape_policy = "deny"
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert_eq!(
            merged["workspace"]["escape_policy"].as_str().unwrap(),
            "deny"
        );
    }

    #[test]
    fn test_never_allow_union() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [workspace]
            never_allow = ["/etc", "/var"]
        "#,
        )
        .unwrap();

        // Workspace tries to remove /etc.
        let workspace: toml::Value = toml::from_str(
            r#"
            [workspace]
            never_allow = ["/var"]
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let arr = merged["workspace"]["never_allow"].as_array().unwrap();
        let strs: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
        assert!(strs.contains(&"/etc"));
        assert!(strs.contains(&"/var"));
    }

    #[test]
    fn test_require_signatures_cannot_disable() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [security]
            require_signatures = true
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [security]
            require_signatures = false
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert!(merged["security"]["require_signatures"].as_bool().unwrap());
    }

    #[test]
    fn test_approval_timeout_cannot_increase() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [security]
            approval_timeout_secs = 300
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [security]
            approval_timeout_secs = 9999
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert_eq!(
            merged["security"]["approval_timeout_secs"]
                .as_integer()
                .unwrap(),
            300
        );
    }

    #[test]
    fn test_approval_required_tools_union() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [security.policy]
            approval_required_tools = ["delete", "exec"]
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [security.policy]
            approval_required_tools = ["exec"]
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let arr = merged["security"]["policy"]["approval_required_tools"]
            .as_array()
            .unwrap();
        let strs: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
        assert!(strs.contains(&"delete"));
        assert!(strs.contains(&"exec"));
    }

    #[test]
    fn test_allowed_paths_cannot_expand() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [security.policy]
            allowed_paths = ["/home/user"]
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [security.policy]
            allowed_paths = ["/home/user", "/etc/secrets"]
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let arr = merged["security"]["policy"]["allowed_paths"]
            .as_array()
            .unwrap();
        let strs: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
        assert!(strs.contains(&"/home/user"));
        assert!(!strs.contains(&"/etc/secrets"));
    }

    #[test]
    fn test_allowed_hosts_cannot_expand() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [security.policy]
            allowed_hosts = []
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [security.policy]
            allowed_hosts = ["evil.com"]
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let arr = merged["security"]["policy"]["allowed_hosts"]
            .as_array()
            .unwrap();
        assert!(arr.is_empty());
    }

    #[test]
    fn test_api_key_cannot_be_overridden_by_workspace() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [model]
            api_key = "sk-real-key"
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [model]
            api_key = "sk-malicious-key"
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert_eq!(merged["model"]["api_key"].as_str().unwrap(), "sk-real-key");
    }

    #[test]
    fn test_api_url_cannot_be_overridden_by_workspace() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [model]
            provider = "claude"
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [model]
            api_url = "https://evil-proxy.com"
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        // api_url should have been removed since baseline didn't have it.
        assert!(merged["model"].as_table().unwrap().get("api_url").is_none());
    }

    #[test]
    fn test_allow_wasm_hooks_cannot_enable() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [hooks]
            allow_wasm_hooks = false
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [hooks]
            allow_wasm_hooks = true
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert!(!merged["hooks"]["allow_wasm_hooks"].as_bool().unwrap());
    }

    #[test]
    fn test_allow_agent_hooks_cannot_enable() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [hooks]
            allow_agent_hooks = false
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [hooks]
            allow_agent_hooks = true
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert!(!merged["hooks"]["allow_agent_hooks"].as_bool().unwrap());
    }

    #[test]
    fn test_rate_limits_cannot_increase() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [rate_limits]
            elicitation_per_server_per_min = 10
            max_pending_requests = 50
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [rate_limits]
            elicitation_per_server_per_min = 100
            max_pending_requests = 500
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert_eq!(
            merged["rate_limits"]["elicitation_per_server_per_min"]
                .as_integer()
                .unwrap(),
            10
        );
        assert_eq!(
            merged["rate_limits"]["max_pending_requests"]
                .as_integer()
                .unwrap(),
            50
        );
    }

    #[test]
    fn test_warn_at_percent_cannot_increase() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [budget]
            warn_at_percent = 80
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [budget]
            warn_at_percent = 99
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert_eq!(
            merged["budget"]["warn_at_percent"].as_integer().unwrap(),
            80
        );
    }

    // ---- Step 4: Workspace server injection ----

    #[test]
    fn test_workspace_server_forced_untrusted() {
        let baseline: toml::Value = toml::from_str("[servers]").unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [servers.evil]
            command = "evil-server"
            trusted = true
            auto_start = true
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let evil = &merged["servers"]["evil"];
        assert!(!evil["trusted"].as_bool().unwrap());
    }

    #[test]
    fn test_workspace_server_forced_no_autostart() {
        let baseline: toml::Value = toml::from_str("[servers]").unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [servers.evil]
            command = "evil-server"
            trusted = false
            auto_start = true
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let evil = &merged["servers"]["evil"];
        assert!(!evil["auto_start"].as_bool().unwrap());
    }

    #[test]
    fn test_existing_server_keeps_trusted() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [servers.mydb]
            command = "db-server"
            trusted = true
            auto_start = true
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [servers.mydb]
            args = ["--verbose"]
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let mydb = &merged["servers"]["mydb"];
        assert!(mydb["trusted"].as_bool().unwrap());
        assert!(mydb["auto_start"].as_bool().unwrap());
    }

    // ---- A1: Workspace cannot change security-critical fields on baseline servers ----

    #[test]
    fn test_workspace_cannot_change_baseline_server_command() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [servers.mydb]
            command = "safe-server"
            trusted = true
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [servers.mydb]
            command = "evil-server"
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let mydb = &merged["servers"]["mydb"];
        assert_eq!(mydb["command"].as_str().unwrap(), "safe-server");
        assert!(mydb["trusted"].as_bool().unwrap());
    }

    #[test]
    fn test_workspace_cannot_change_baseline_server_args() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [servers.mydb]
            command = "safe-server"
            args = ["--safe"]
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [servers.mydb]
            args = ["--evil-flag"]
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let mydb = &merged["servers"]["mydb"];
        let args = mydb["args"].as_array().unwrap();
        assert_eq!(args[0].as_str().unwrap(), "--safe");
    }

    #[test]
    fn test_workspace_cannot_change_baseline_server_trusted() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [servers.mydb]
            command = "safe-server"
            trusted = false
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [servers.mydb]
            trusted = true
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let mydb = &merged["servers"]["mydb"];
        assert!(!mydb["trusted"].as_bool().unwrap());
    }

    #[test]
    fn test_workspace_can_add_non_protected_fields_to_baseline_server() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [servers.mydb]
            command = "safe-server"
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [servers.mydb]
            auto_start = true
            transport = "stdio"
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let mydb = &merged["servers"]["mydb"];
        // Non-protected fields should be allowed
        assert_eq!(mydb["transport"].as_str().unwrap(), "stdio");
        // auto_start is not in the protected list for baseline servers
        assert!(mydb["auto_start"].as_bool().unwrap());
    }

    // ---- A2: auto_allow_read/write cannot expand beyond baseline ----

    #[test]
    fn test_auto_allow_write_cannot_expand() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [workspace]
            auto_allow_write = []
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [workspace]
            auto_allow_write = ["/**"]
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let arr = merged["workspace"]["auto_allow_write"].as_array().unwrap();
        assert!(arr.is_empty());
    }

    #[test]
    fn test_auto_allow_read_cannot_expand() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [workspace]
            auto_allow_read = ["/usr/share"]
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [workspace]
            auto_allow_read = ["/usr/share", "/etc/secrets"]
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        let arr = merged["workspace"]["auto_allow_read"].as_array().unwrap();
        let strs: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
        assert!(strs.contains(&"/usr/share"));
        assert!(!strs.contains(&"/etc/secrets"));
    }

    // ---- idle_secs, allow_http_hooks, allow_command_hooks restrictions ----

    #[test]
    fn test_idle_secs_cannot_increase() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [timeouts]
            idle_secs = 3600
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [timeouts]
            idle_secs = 86400
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert_eq!(merged["timeouts"]["idle_secs"].as_integer().unwrap(), 3600);
    }

    #[test]
    fn test_idle_secs_can_decrease() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [timeouts]
            idle_secs = 3600
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [timeouts]
            idle_secs = 600
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert_eq!(merged["timeouts"]["idle_secs"].as_integer().unwrap(), 600);
    }

    #[test]
    fn test_allow_http_hooks_cannot_enable() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [hooks]
            allow_http_hooks = false
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [hooks]
            allow_http_hooks = true
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert!(!merged["hooks"]["allow_http_hooks"].as_bool().unwrap());
    }

    #[test]
    fn test_allow_command_hooks_cannot_enable() {
        let baseline: toml::Value = toml::from_str(
            r#"
            [hooks]
            allow_command_hooks = false
        "#,
        )
        .unwrap();

        let workspace: toml::Value = toml::from_str(
            r#"
            [hooks]
            allow_command_hooks = true
        "#,
        )
        .unwrap();

        let mut merged = baseline.clone();
        deep_merge(&mut merged, &workspace);
        enforce_restrictions(&mut merged, &baseline, &workspace);

        assert!(!merged["hooks"]["allow_command_hooks"].as_bool().unwrap());
    }

    // ---- Step 7: Robustness ----

    #[test]
    fn test_set_nested_no_panic_on_missing_table() {
        let mut val: toml::Value = toml::from_str("[model]\nprovider = \"claude\"").unwrap();
        // This should not panic — the intermediate "nonexistent" table is missing.
        set_nested(
            &mut val,
            &["nonexistent", "field"],
            toml::Value::Boolean(true),
        );
        // Value should be unchanged.
        assert_eq!(val["model"]["provider"].as_str().unwrap(), "claude");
    }
}
