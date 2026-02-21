use crate::error::{PluginError, PluginResult};

/// Validate that a URL uses an allowed scheme.
///
/// # Errors
///
/// Returns an error if the URL scheme is not one of the explicitly allowed schemes (e.g. `https://`, `ssh://`).
pub fn validate_url_scheme(url: &str) -> PluginResult<()> {
    let allowed = ["https://", "ssh://"];
    if allowed.iter().any(|scheme| url.starts_with(scheme)) {
        return Ok(());
    }
    Err(PluginError::ExecutionFailed(format!(
        "blocked URL scheme in '{url}'. Only https:// and ssh:// are allowed"
    )))
}

/// Validate an SSH hostname for safety.
///
/// Rejects control characters, spaces, slashes, brackets (`IPv6`), and other
/// characters that could cause URL confusion or injection.
///
/// # Errors
///
/// Returns an error if the hostname contains invalid characters or has an invalid format.
pub fn validate_ssh_host(host: &str) -> PluginResult<()> {
    let is_valid = host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'));
    if !is_valid {
        return Err(PluginError::ExecutionFailed(format!(
            "SSH host contains invalid characters: '{host}'. Only alphanumeric, hyphens, and dots are allowed"
        )));
    }
    if host.starts_with('-') || host.starts_with('.') || host.ends_with('.') {
        return Err(PluginError::ExecutionFailed(format!(
            "SSH host has invalid format: '{host}'"
        )));
    }
    Ok(())
}

/// Validate an SSH path component for safety.
///
/// Rejects path traversal (`..`), control characters, and other dangerous patterns.
///
/// # Errors
///
/// Returns an error if the path contains `..` or forbidden control characters.
pub fn validate_ssh_path(path: &str) -> PluginResult<()> {
    if path.contains("..") {
        return Err(PluginError::ExecutionFailed(format!(
            "SSH path contains '..': '{path}'"
        )));
    }
    let has_bad_chars = path
        .bytes()
        .any(|b| b.is_ascii_control() || matches!(b, b' ' | b'\\' | b':'));
    if has_bad_chars {
        return Err(PluginError::ExecutionFailed(format!(
            "SSH path contains invalid characters: '{path}'"
        )));
    }
    Ok(())
}

/// Validate a GitHub org or repo component against injection attacks.
///
/// GitHub usernames/org names: alphanumeric + hyphens (max 39 chars).
/// Repo names: alphanumeric + hyphens + underscores + dots (max 100 chars).
/// We use a generous superset that covers both.
///
/// # Errors
///
/// Returns an error if the component is empty, exceeds length limits, or contains invalid characters.
pub fn validate_github_component(value: &str, label: &str) -> PluginResult<()> {
    if value.is_empty() || value.len() > 100 {
        return Err(PluginError::ExecutionFailed(format!(
            "GitHub {label} must be 1-100 characters, got {}",
            value.len()
        )));
    }
    let is_valid = value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if !is_valid {
        return Err(PluginError::ExecutionFailed(format!(
            "GitHub {label} contains invalid characters: '{value}'"
        )));
    }
    // Reject patterns GitHub disallows or that could cause path/arg issues
    if value.starts_with('.')
        || value.starts_with('-')
        || value.ends_with('.')
        || value.contains("..")
    {
        return Err(PluginError::ExecutionFailed(format!(
            "GitHub {label} has invalid format: '{value}'"
        )));
    }
    Ok(())
}

/// Validate a git ref (branch, tag, or commit) for safety.
///
/// Rejects control characters, path traversal (`..`), shell metacharacters,
/// and enforces git naming rules.
///
/// # Errors
///
/// Returns an error if the ref is empty, exceeds length limits, or violates git ref naming rules.
pub fn validate_git_ref(git_ref: &str) -> PluginResult<()> {
    if git_ref.is_empty() || git_ref.len() > 256 {
        return Err(PluginError::ExecutionFailed(
            "git ref must be 1-256 characters".into(),
        ));
    }
    if git_ref.contains("..") {
        return Err(PluginError::ExecutionFailed(format!(
            "git ref contains '..': '{git_ref}'"
        )));
    }
    if git_ref.starts_with('-') {
        return Err(PluginError::ExecutionFailed(format!(
            "git ref must not start with '-': '{git_ref}'"
        )));
    }
    // Only allow characters valid in git branch/tag names
    let is_valid = git_ref
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'));
    if !is_valid {
        return Err(PluginError::ExecutionFailed(format!(
            "git ref contains invalid characters: '{git_ref}'"
        )));
    }
    // Git doesn't allow refs starting/ending with '.' or '/', ending with '.lock',
    // or containing consecutive slashes
    if git_ref.starts_with('.')
        || git_ref.ends_with('.')
        || git_ref.starts_with('/')
        || git_ref.ends_with('/')
        || std::path::Path::new(git_ref)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("lock"))
        || git_ref.contains("//")
    {
        return Err(PluginError::ExecutionFailed(format!(
            "git ref has invalid format: '{git_ref}'"
        )));
    }
    Ok(())
}
