use super::{ConfigLayer, FieldSources};

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
