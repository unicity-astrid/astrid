use tracing::warn;

/// Navigate into a nested `toml::Value` by dotted path segments.
pub(super) fn get_nested<'a>(val: &'a toml::Value, path: &[&str]) -> Option<&'a toml::Value> {
    let mut current = val;
    for segment in path {
        current = current.as_table()?.get(*segment)?;
    }
    Some(current)
}

/// Set a value at a nested path, creating intermediate tables as needed.
pub(super) fn set_nested(val: &mut toml::Value, path: &[&str], new_val: toml::Value) {
    if path.is_empty() {
        return;
    }

    let mut current = val;
    // Safety: path is non-empty (checked above)
    #[allow(clippy::arithmetic_side_effects)]
    let parent_path = &path[..path.len() - 1];
    for segment in parent_path {
        let Some(next) = current.as_table_mut().and_then(|t| t.get_mut(*segment)) else {
            warn!("set_nested: missing intermediate table at '{segment}'; skipping");
            return;
        };
        current = next;
    }

    if let Some(table) = current.as_table_mut() {
        // Safety: path is non-empty (checked above)
        #[allow(clippy::arithmetic_side_effects)]
        let leaf = path[path.len() - 1];
        table.insert(leaf.to_owned(), new_val);
    }
}

/// Remove a value at a nested path.
pub(super) fn remove_nested(val: &mut toml::Value, path: &[&str]) {
    if path.is_empty() {
        return;
    }

    let mut current = val;
    // Safety: path is non-empty (checked above)
    #[allow(clippy::arithmetic_side_effects)]
    let parent_path = &path[..path.len() - 1];
    for segment in parent_path {
        let Some(next) = current.as_table_mut().and_then(|t| t.get_mut(*segment)) else {
            return;
        };
        current = next;
    }

    if let Some(table) = current.as_table_mut() {
        // Safety: path is non-empty (checked above)
        #[allow(clippy::arithmetic_side_effects)]
        let leaf = path[path.len() - 1];
        table.remove(leaf);
    }
}
