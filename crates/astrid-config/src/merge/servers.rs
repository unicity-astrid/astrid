use tracing::warn;

use super::path::{get_nested, remove_nested, set_nested};

/// Security-critical fields that a workspace must not change on baseline servers.
pub(super) const PROTECTED_SERVER_FIELDS: &[&str] =
    &["command", "args", "env", "cwd", "binary_hash", "trusted"];

/// Prevent workspace-injected servers from being trusted or auto-started,
/// and protect security-critical fields on baseline servers.
pub(super) fn sanitize_workspace_servers(
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
