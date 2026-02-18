use tracing::warn;

use super::path::{get_nested, remove_nested, set_nested};

/// Clamp a float field so workspace cannot increase it beyond baseline.
pub(super) fn clamp_max(
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
pub(super) fn clamp_max_int(
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
pub(super) fn enforce_bool_only_true(
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
pub(super) fn enforce_bool_only_false(
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
pub(super) fn union_string_arrays(
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
pub(super) fn enforce_mode_tighten(
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
pub(super) fn block_workspace_override(
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
pub(super) fn block_workspace_expansion(
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
