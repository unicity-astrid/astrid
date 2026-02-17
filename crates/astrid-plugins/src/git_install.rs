//! Git-based plugin installation.
//!
//! Supports two source formats:
//! - `github:org/repo[@ref]` — fetches via GitHub tarball API
//! - `git:https://host/path.git[@ref]` — clones via `git clone --depth=1`
//!
//! After fetching, the source is extracted into a temporary directory and
//! returned for the caller to detect the plugin type and route to the
//! appropriate install pipeline.

use std::path::PathBuf;
use std::process::Command;

use crate::error::{PluginError, PluginResult};

/// Maximum tarball download size (100 MB).
const MAX_DOWNLOAD_SIZE: u64 = 100 * 1024 * 1024;

/// Timeout for git clone operations (5 minutes).
const GIT_CLONE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Parsed git source specifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitSource {
    /// GitHub shorthand: `github:org/repo[@ref]`.
    GitHub {
        /// GitHub organization or user.
        org: String,
        /// Repository name.
        repo: String,
        /// Optional git ref (tag, branch, or commit).
        git_ref: Option<String>,
    },
    /// Generic git URL: `git:https://host/path.git[@ref]`.
    GitUrl {
        /// Full repository URL (must be `https://` or `ssh://`).
        url: String,
        /// Optional git ref.
        git_ref: Option<String>,
    },
}

impl GitSource {
    /// Parse a git source specifier string.
    ///
    /// Accepted formats:
    /// - `github:org/repo`
    /// - `github:org/repo@v1.0.0`
    /// - `git:https://gitlab.com/org/repo.git`
    /// - `git:https://gitlab.com/org/repo.git@main`
    ///
    /// # Errors
    ///
    /// Returns an error for invalid format or blocked URL schemes.
    pub fn parse(source: &str) -> PluginResult<Self> {
        if let Some(rest) = source.strip_prefix("github:") {
            return Self::parse_github(rest);
        }
        if let Some(rest) = source.strip_prefix("git:") {
            return Self::parse_git_url(rest);
        }
        // Bare HTTPS URLs pointing to known git hosts or ending in .git
        if Self::looks_like_bare_https(source) {
            return Self::parse_git_url(source);
        }
        // SSH URLs: git@host:org/repo
        if source.starts_with("git@") {
            return Self::parse_ssh_url(source);
        }
        Err(PluginError::ExecutionFailed(format!(
            "invalid git source: '{source}'. Expected 'github:org/repo[@ref]', 'git:URL[@ref]', or a git URL (https/ssh)"
        )))
    }

    /// Check if a source string looks like a git URL (any supported format).
    #[must_use]
    pub fn looks_like_git(source: &str) -> bool {
        source.starts_with("github:")
            || source.starts_with("git:")
            || source.starts_with("git@")
            || Self::looks_like_bare_https(source)
    }

    /// Check if a source looks like a bare `https://` git URL.
    ///
    /// Matches when the **host** is a known git forge (github.com, gitlab.com)
    /// or the URL path ends in `.git` (after stripping any `@ref` suffix).
    fn looks_like_bare_https(source: &str) -> bool {
        let Some(after_scheme) = source.strip_prefix("https://") else {
            return false;
        };
        // Extract host: everything before the first '/'
        let host = after_scheme.split('/').next().unwrap_or("");
        if host.eq_ignore_ascii_case("github.com") || host.eq_ignore_ascii_case("gitlab.com") {
            return true;
        }
        // Strip @ref suffix before checking extension, so
        // "https://host/repo.git@main" is recognized correctly.
        let (url_part, _) = split_ref(source);
        std::path::Path::new(url_part.as_str())
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("git"))
    }

    /// Derive a plugin ID from the git source.
    ///
    /// Uses the repo name (lowercase, hyphens only) as the plugin ID.
    #[must_use]
    pub fn plugin_id_hint(&self) -> String {
        let raw = match self {
            Self::GitHub { repo, .. } => repo.clone(),
            Self::GitUrl { url, .. } => {
                // Extract last path segment, strip .git suffix
                url.trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or("plugin")
                    .trim_end_matches(".git")
                    .to_string()
            },
        };
        // Sanitize: lowercase, only alphanumeric and hyphens
        let hint: String = raw
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        if hint.is_empty() || hint.chars().all(|c| c == '-') {
            "git-plugin".to_string()
        } else {
            hint
        }
    }

    /// Get a display string for the source (used in lockfile entries).
    #[must_use]
    pub fn display_source(&self) -> String {
        match self {
            Self::GitHub {
                org,
                repo,
                git_ref: None,
            } => format!("github:{org}/{repo}"),
            Self::GitHub {
                org,
                repo,
                git_ref: Some(r),
            } => format!("github:{org}/{repo}@{r}"),
            Self::GitUrl { url, git_ref: None } => format!("git:{url}"),
            Self::GitUrl {
                url,
                git_ref: Some(r),
            } => format!("git:{url}@{r}"),
        }
    }

    fn parse_github(rest: &str) -> PluginResult<Self> {
        let (path, git_ref) = split_ref(rest);

        let parts: Vec<&str> = path.splitn(2, '/').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err(PluginError::ExecutionFailed(format!(
                "invalid GitHub specifier: '{rest}'. Expected 'org/repo[@ref]'"
            )));
        }

        // Validate org/repo to prevent URL injection in API requests
        validate_github_component(parts[0], "org")?;
        validate_github_component(parts[1], "repo")?;

        if let Some(ref r) = git_ref {
            validate_git_ref(r)?;
        }

        Ok(Self::GitHub {
            org: parts[0].to_string(),
            repo: parts[1].to_string(),
            git_ref,
        })
    }

    fn parse_git_url(rest: &str) -> PluginResult<Self> {
        let (url, git_ref) = split_ref(rest);

        // URL scheme whitelist: only https:// and ssh://
        validate_url_scheme(&url)?;

        if let Some(ref r) = git_ref {
            validate_git_ref(r)?;
        }

        Ok(Self::GitUrl { url, git_ref })
    }

    /// Parse SSH-style `git@host:org/repo[.git][@ref]` into a `GitUrl`.
    ///
    /// Converts the SCP-style syntax to `ssh://git@host/org/repo` so that
    /// downstream `git clone` works uniformly.
    fn parse_ssh_url(source: &str) -> PluginResult<Self> {
        // Split ref first: git@github.com:org/repo.git@main → (git@github.com:org/repo.git, Some(main))
        let (url_part, git_ref) = split_ssh_ref(source);

        // Expect git@<host>:<path>
        let after_at = url_part
            .strip_prefix("git@")
            .ok_or_else(|| PluginError::ExecutionFailed(format!("invalid SSH URL: '{source}'")))?;

        let (host, path) = after_at.split_once(':').ok_or_else(|| {
            PluginError::ExecutionFailed(format!(
                "invalid SSH URL: '{source}'. Expected 'git@host:org/repo'"
            ))
        })?;

        if host.is_empty() || path.is_empty() {
            return Err(PluginError::ExecutionFailed(format!(
                "invalid SSH URL: '{source}'. Expected 'git@host:org/repo'"
            )));
        }

        validate_ssh_host(host)?;
        validate_ssh_path(path)?;

        let url = format!("ssh://git@{host}/{path}");

        // Defense-in-depth: verify the constructed URL passes scheme validation
        validate_url_scheme(&url)?;

        if let Some(ref r) = git_ref {
            validate_git_ref(r)?;
        }

        Ok(Self::GitUrl { url, git_ref })
    }
}

/// Split a `value@ref` string into `(value, Option<ref>)`.
///
/// For git URLs, the `@` delimiter is only recognized in the path portion
/// (after the first `/` past `://`) to avoid splitting on `@` in the
/// authority of SSH URLs like `ssh://git@github.com/...`.
fn split_ref(s: &str) -> (String, Option<String>) {
    // For URLs containing "://", only look for "@" in the path portion
    // (after the authority), not in user@host.
    if let Some(scheme_end) = s.find("://") {
        let authority_start = scheme_end.saturating_add(3);
        let after_scheme = &s[authority_start..];
        // Find the first '/' after the authority to skip user@host
        let path_start = after_scheme.find('/').unwrap_or(after_scheme.len());
        let path_portion = &after_scheme[path_start..];
        if let Some(at_pos) = path_portion.rfind('@') {
            let split_pos = authority_start
                .saturating_add(path_start)
                .saturating_add(at_pos);
            let url = s[..split_pos].to_string();
            let ref_start = split_pos.saturating_add(1);
            let git_ref = s[ref_start..].to_string();
            if git_ref.is_empty() {
                return (s.to_string(), None);
            }
            return (url, Some(git_ref));
        }
        return (s.to_string(), None);
    }

    // For non-URL strings (github shorthand), split on the first "@"
    if let Some(at_pos) = s.find('@') {
        let value = s[..at_pos].to_string();
        let ref_start = at_pos.saturating_add(1);
        let git_ref = s[ref_start..].to_string();
        if git_ref.is_empty() {
            return (s.to_string(), None);
        }
        return (value, Some(git_ref));
    }

    (s.to_string(), None)
}

/// Split an SCP-style SSH ref: `git@host:org/repo.git@ref` → `(git@host:org/repo.git, Some(ref))`.
///
/// The first `@` is part of `git@host`, so we look for `@` only after the
/// colon-separated path portion (i.e. after `host:`).
fn split_ssh_ref(s: &str) -> (String, Option<String>) {
    // Find the colon that separates host from path: git@github.com:org/repo
    if let Some((host_part, path)) = s.split_once(':')
        && let Some((path_part, git_ref)) = path.rsplit_once('@')
        && !git_ref.is_empty()
    {
        let url = format!("{host_part}:{path_part}");
        return (url, Some(git_ref.to_string()));
    }
    (s.to_string(), None)
}

/// Validate that a URL uses an allowed scheme.
fn validate_url_scheme(url: &str) -> PluginResult<()> {
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
fn validate_ssh_host(host: &str) -> PluginResult<()> {
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
fn validate_ssh_path(path: &str) -> PluginResult<()> {
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
fn validate_github_component(value: &str, label: &str) -> PluginResult<()> {
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
fn validate_git_ref(git_ref: &str) -> PluginResult<()> {
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

/// Fetch a plugin source from a git repository into a temporary directory.
///
/// Returns the temp dir (ownership transferred to caller) and the path
/// to the extracted/cloned source root.
///
/// # Errors
///
/// Returns an error if the fetch fails, download exceeds size limits,
/// or extraction encounters security violations.
#[cfg(feature = "http")]
pub async fn fetch_git_source(source: &GitSource) -> PluginResult<(tempfile::TempDir, PathBuf)> {
    match source {
        GitSource::GitHub { org, repo, git_ref } => {
            fetch_github_tarball(org, repo, git_ref.as_deref()).await
        },
        GitSource::GitUrl { url, git_ref } => {
            // Run blocking git clone on a dedicated thread with a timeout
            // to avoid blocking the Tokio runtime and prevent indefinite hangs.
            let url = url.clone();
            let git_ref = git_ref.clone();
            tokio::time::timeout(
                GIT_CLONE_TIMEOUT,
                tokio::task::spawn_blocking(move || clone_git_repo(&url, git_ref.as_deref())),
            )
            .await
            .map_err(|_| {
                PluginError::ExecutionFailed(format!(
                    "git clone timed out after {}s",
                    GIT_CLONE_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|e| PluginError::ExecutionFailed(format!("git clone task panicked: {e}")))?
        },
    }
}

/// Fetch a GitHub repository as a tarball via the API.
#[cfg(feature = "http")]
async fn fetch_github_tarball(
    org: &str,
    repo: &str,
    git_ref: Option<&str>,
) -> PluginResult<(tempfile::TempDir, PathBuf)> {
    let ref_part = git_ref.unwrap_or("HEAD");
    let url = format!("https://api.github.com/repos/{org}/{repo}/tarball/{ref_part}");

    tracing::debug!("Fetching GitHub tarball: {url}");

    let client = reqwest::Client::builder()
        .user_agent("astrid-plugin-installer")
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| PluginError::ExecutionFailed(format!("failed to create HTTP client: {e}")))?;

    let response =
        client.get(&url).send().await.map_err(|e| {
            PluginError::ExecutionFailed(format!("GitHub tarball fetch failed: {e}"))
        })?;

    if !response.status().is_success() {
        return Err(PluginError::ExecutionFailed(format!(
            "GitHub API returned {}: {org}/{repo}@{ref_part}",
            response.status()
        )));
    }

    // Check Content-Length if available
    if let Some(len) = response.content_length()
        && len > MAX_DOWNLOAD_SIZE
    {
        return Err(PluginError::PackageTooLarge {
            size: len,
            limit: MAX_DOWNLOAD_SIZE,
        });
    }

    // Stream the response body with size limit
    let bytes = download_with_limit(response, MAX_DOWNLOAD_SIZE).await?;

    // Extract the tarball
    let tmp = tempfile::tempdir()
        .map_err(|e| PluginError::ExecutionFailed(format!("failed to create temp dir: {e}")))?;

    let root = extract_github_tarball(&bytes, tmp.path())?;

    Ok((tmp, root))
}

/// Download a response body with a size limit.
#[cfg(feature = "http")]
async fn download_with_limit(response: reqwest::Response, max_size: u64) -> PluginResult<Vec<u8>> {
    use futures::StreamExt;

    let capacity =
        usize::try_from(response.content_length().unwrap_or(0).min(max_size)).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|e| PluginError::ExecutionFailed(format!("download error: {e}")))?;
        bytes.extend_from_slice(&chunk);
        let current_size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if current_size > max_size {
            return Err(PluginError::PackageTooLarge {
                size: current_size,
                limit: max_size,
            });
        }
    }

    Ok(bytes)
}

/// Extract a GitHub tarball (gzip-compressed tar).
///
/// GitHub tarballs have a leading `{org}-{repo}-{sha}/` directory.
/// This function strips the first directory component generically.
#[cfg(feature = "http")]
#[allow(clippy::too_many_lines)]
fn extract_github_tarball(data: &[u8], dest: &std::path::Path) -> PluginResult<PathBuf> {
    const MAX_ENTRY_COUNT: usize = 10_000;
    const MAX_EXTRACTED_SIZE: u64 = 500_000_000;

    let decoder = flate2::read::GzDecoder::new(data);
    let mut archive = tar::Archive::new(decoder);
    // Disable permission preservation to prevent setuid/setgid bits
    // from malicious tarballs being restored on extracted files.
    archive.set_preserve_permissions(false);

    let dest = dest
        .canonicalize()
        .map_err(|e| PluginError::ExtractionError {
            message: format!("failed to canonicalize destination: {e}"),
        })?;

    let mut entry_count = 0usize;
    let mut total_size: u64 = 0;

    for entry_result in archive
        .entries()
        .map_err(|e| PluginError::ExtractionError {
            message: format!("failed to read archive entries: {e}"),
        })?
    {
        let mut entry = entry_result.map_err(|e| PluginError::ExtractionError {
            message: format!("failed to read archive entry: {e}"),
        })?;

        entry_count = entry_count.saturating_add(1);
        if entry_count > MAX_ENTRY_COUNT {
            return Err(PluginError::ExtractionError {
                message: format!("archive exceeds maximum entry count ({MAX_ENTRY_COUNT})"),
            });
        }

        // Reject unsafe entry types
        let entry_type = entry.header().entry_type();
        if !matches!(
            entry_type,
            tar::EntryType::Regular
                | tar::EntryType::Directory
                | tar::EntryType::GNULongName
                | tar::EntryType::XHeader
                | tar::EntryType::XGlobalHeader
        ) {
            let entry_path = entry
                .path()
                .map_or_else(|_| "<unknown>".to_string(), |p| p.display().to_string());
            return Err(PluginError::UnsafeEntryType {
                entry_type: format!("{entry_type:?}"),
                path: entry_path,
            });
        }

        let entry_size = entry
            .header()
            .size()
            .map_err(|e| PluginError::ExtractionError {
                message: format!("failed to read entry size: {e}"),
            })?;
        total_size = total_size.saturating_add(entry_size);
        if total_size > MAX_EXTRACTED_SIZE {
            return Err(PluginError::ExtractionError {
                message: format!(
                    "archive exceeds maximum extracted size ({MAX_EXTRACTED_SIZE} bytes)"
                ),
            });
        }

        let entry_path = entry
            .path()
            .map_err(|e| PluginError::ExtractionError {
                message: format!("failed to read entry path: {e}"),
            })?
            .into_owned();

        // Validate path safety
        if entry_path.is_absolute() {
            return Err(PluginError::PathTraversal {
                path: entry_path.display().to_string(),
            });
        }
        for component in entry_path.components() {
            if matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::Prefix(_)
                    | std::path::Component::RootDir
            ) {
                return Err(PluginError::PathTraversal {
                    path: entry_path.display().to_string(),
                });
            }
        }

        // Strip the first directory component (GitHub's `{org}-{repo}-{sha}/`)
        let stripped = strip_first_component(&entry_path);
        let target = dest.join(stripped);

        // First: lexical boundary check (before creating any dirs).
        // dest is already canonicalized (line 389), so this catches obvious escapes.
        if !target.starts_with(&dest) {
            return Err(PluginError::PathTraversal {
                path: entry_path.display().to_string(),
            });
        }

        // Create parent directories
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PluginError::ExtractionError {
                message: format!("failed to create directory {}: {e}", parent.display()),
            })?;
        }

        // Second: symlink-aware boundary check (after dirs exist, so canonicalize succeeds).
        // This catches symlink-based escapes that the lexical check misses.
        let canonical_parent = target
            .parent()
            .ok_or_else(|| PluginError::PathTraversal {
                path: entry_path.display().to_string(),
            })?
            .canonicalize()
            .map_err(|e| PluginError::ExtractionError {
                message: format!("failed to canonicalize path for boundary check: {e}"),
            })?;
        let canonical_target = canonical_parent.join(target.file_name().unwrap_or_default());
        if !canonical_target.starts_with(&dest) {
            return Err(PluginError::PathTraversal {
                path: entry_path.display().to_string(),
            });
        }

        entry
            .unpack(&target)
            .map_err(|e| PluginError::ExtractionError {
                message: format!("failed to unpack {}: {e}", entry_path.display()),
            })?;
    }

    if entry_count == 0 {
        return Err(PluginError::ExtractionError {
            message: "archive is empty".into(),
        });
    }

    Ok(dest)
}

/// Strip the first path component (e.g., `org-repo-sha/src/main.js` → `src/main.js`).
fn strip_first_component(path: &std::path::Path) -> PathBuf {
    let mut components = path.components();
    components.next(); // skip first
    components.as_path().to_path_buf()
}

/// Clone a git repository into a temporary directory.
///
/// Suppresses interactive credential prompts and pipes stdin to null
/// to prevent hanging if authentication is required.
fn clone_git_repo(url: &str, git_ref: Option<&str>) -> PluginResult<(tempfile::TempDir, PathBuf)> {
    let tmp = tempfile::tempdir()
        .map_err(|e| PluginError::ExecutionFailed(format!("failed to create temp dir: {e}")))?;

    let clone_path = tmp.path().join("repo");
    let mut cmd = Command::new("git");

    // Clear inherited environment to prevent injected command execution via
    // GIT_PROXY_COMMAND, GIT_EXTERNAL_DIFF, GIT_CONFIG_GLOBAL, etc.
    cmd.env_clear();
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    // Prevent code execution via ~/.gitconfig directives (core.fsmonitor,
    // core.hooksPath, etc.) while still allowing SSH key lookup via HOME.
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
    // Suppress interactive credential prompts — fail fast if auth is needed.
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    cmd.stdin(std::process::Stdio::null());

    cmd.args(["clone", "--depth=1"]);

    if let Some(r) = git_ref {
        cmd.args(["--branch", r]);
    }

    cmd.arg(url);
    cmd.arg(&clone_path);

    let output = cmd
        .output()
        .map_err(|e| PluginError::ExecutionFailed(format!("failed to run git clone: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PluginError::ExecutionFailed(format!(
            "git clone failed:\n{stderr}"
        )));
    }

    Ok((tmp, clone_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_github_simple() {
        let src = GitSource::parse("github:unicitynetwork/openclaw-unicity").unwrap();
        assert_eq!(
            src,
            GitSource::GitHub {
                org: "unicitynetwork".to_string(),
                repo: "openclaw-unicity".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_github_with_ref() {
        let src = GitSource::parse("github:unicitynetwork/openclaw-unicity@v0.3.9").unwrap();
        assert_eq!(
            src,
            GitSource::GitHub {
                org: "unicitynetwork".to_string(),
                repo: "openclaw-unicity".to_string(),
                git_ref: Some("v0.3.9".to_string()),
            }
        );
    }

    #[test]
    fn parse_git_url_https() {
        let src = GitSource::parse("git:https://gitlab.com/org/repo.git").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://gitlab.com/org/repo.git".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_git_url_with_ref() {
        let src = GitSource::parse("git:https://gitlab.com/org/repo.git@main").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://gitlab.com/org/repo.git".to_string(),
                git_ref: Some("main".to_string()),
            }
        );
    }

    #[test]
    fn parse_git_url_ssh() {
        let src = GitSource::parse("git:ssh://git@github.com/org/repo.git").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "ssh://git@github.com/org/repo.git".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn reject_file_scheme() {
        let err = GitSource::parse("git:file:///etc/passwd").unwrap_err();
        assert!(err.to_string().contains("blocked URL scheme"));
    }

    #[test]
    fn reject_javascript_scheme() {
        let err = GitSource::parse("git:javascript:alert(1)").unwrap_err();
        assert!(err.to_string().contains("blocked URL scheme"));
    }

    #[test]
    fn reject_invalid_format() {
        let err = GitSource::parse("npm:some-package").unwrap_err();
        assert!(err.to_string().contains("invalid git source"));
    }

    #[test]
    fn reject_empty_org_or_repo() {
        assert!(GitSource::parse("github:/repo").is_err());
        assert!(GitSource::parse("github:org/").is_err());
        assert!(GitSource::parse("github:noslash").is_err());
    }

    #[test]
    fn plugin_id_hint_github() {
        let src = GitSource::GitHub {
            org: "unicity".to_string(),
            repo: "openclaw-unicity".to_string(),
            git_ref: None,
        };
        assert_eq!(src.plugin_id_hint(), "openclaw-unicity");
    }

    #[test]
    fn plugin_id_hint_git_url() {
        let src = GitSource::GitUrl {
            url: "https://gitlab.com/org/My_Plugin.git".to_string(),
            git_ref: None,
        };
        assert_eq!(src.plugin_id_hint(), "my-plugin");
    }

    #[test]
    fn display_source_github() {
        let src = GitSource::GitHub {
            org: "org".to_string(),
            repo: "repo".to_string(),
            git_ref: Some("v1.0".to_string()),
        };
        assert_eq!(src.display_source(), "github:org/repo@v1.0");
    }

    #[test]
    fn display_source_git_url() {
        let src = GitSource::GitUrl {
            url: "https://example.com/repo.git".to_string(),
            git_ref: None,
        };
        assert_eq!(src.display_source(), "git:https://example.com/repo.git");
    }

    #[test]
    fn strip_first_component_works() {
        let p = std::path::Path::new("org-repo-abc123/src/main.js");
        let stripped = strip_first_component(p);
        assert_eq!(stripped, PathBuf::from("src/main.js"));
    }

    #[test]
    fn strip_first_component_single() {
        let p = std::path::Path::new("only-one");
        let stripped = strip_first_component(p);
        assert_eq!(stripped, PathBuf::from(""));
    }

    #[test]
    fn validate_url_scheme_allowed() {
        assert!(validate_url_scheme("https://github.com/org/repo").is_ok());
        assert!(validate_url_scheme("ssh://git@github.com/org/repo").is_ok());
    }

    #[test]
    fn validate_url_scheme_blocked() {
        assert!(validate_url_scheme("file:///etc/passwd").is_err());
        assert!(validate_url_scheme("http://insecure.com/repo").is_err());
        assert!(validate_url_scheme("ftp://files.com/repo").is_err());
    }

    #[test]
    fn split_ref_ssh_url_without_ref() {
        // The '@' in ssh://git@github.com must NOT be treated as a ref delimiter
        let (url, git_ref) = split_ref("ssh://git@github.com/org/repo.git");
        assert_eq!(url, "ssh://git@github.com/org/repo.git");
        assert_eq!(git_ref, None);
    }

    #[test]
    fn split_ref_ssh_url_with_ref() {
        let (url, git_ref) = split_ref("ssh://git@github.com/org/repo.git@v1.0.0");
        assert_eq!(url, "ssh://git@github.com/org/repo.git");
        assert_eq!(git_ref, Some("v1.0.0".to_string()));
    }

    #[test]
    fn split_ref_https_url_with_ref() {
        let (url, git_ref) = split_ref("https://github.com/org/repo.git@main");
        assert_eq!(url, "https://github.com/org/repo.git");
        assert_eq!(git_ref, Some("main".to_string()));
    }

    #[test]
    fn split_ref_https_url_without_ref() {
        let (url, git_ref) = split_ref("https://github.com/org/repo.git");
        assert_eq!(url, "https://github.com/org/repo.git");
        assert_eq!(git_ref, None);
    }

    #[test]
    fn reject_github_org_with_slashes() {
        assert!(validate_github_component("org/evil", "org").is_err());
    }

    #[test]
    fn reject_github_org_with_url_injection() {
        assert!(validate_github_component("org/../admin", "org").is_err());
    }

    #[test]
    fn reject_github_component_leading_dash() {
        assert!(validate_github_component("-evil", "org").is_err());
    }

    #[test]
    fn reject_github_component_leading_dot() {
        assert!(validate_github_component(".hidden", "org").is_err());
    }

    #[test]
    fn accept_valid_github_components() {
        assert!(validate_github_component("my-org", "org").is_ok());
        assert!(validate_github_component("my.repo_name", "repo").is_ok());
        assert!(validate_github_component("CamelCase123", "org").is_ok());
    }

    #[test]
    fn reject_git_ref_with_double_dot() {
        assert!(validate_git_ref("main..evil").is_err());
    }

    #[test]
    fn reject_git_ref_with_control_chars() {
        assert!(validate_git_ref("main\x00evil").is_err());
        assert!(validate_git_ref("v1.0;rm -rf /").is_err());
    }

    #[test]
    fn reject_git_ref_too_long() {
        let long_ref = "a".repeat(257);
        assert!(validate_git_ref(&long_ref).is_err());
    }

    #[test]
    fn accept_valid_git_refs() {
        assert!(validate_git_ref("main").is_ok());
        assert!(validate_git_ref("v1.0.0").is_ok());
        assert!(validate_git_ref("feature/my-branch").is_ok());
        assert!(validate_git_ref("abc123def456").is_ok());
    }

    #[test]
    fn reject_github_source_with_invalid_org() {
        let err = GitSource::parse("github:org/slashes/not-allowed").unwrap_err();
        assert!(err.to_string().contains("invalid characters"));
    }

    #[test]
    fn reject_github_source_with_bad_ref() {
        let err = GitSource::parse("github:org/repo@main..evil").unwrap_err();
        assert!(err.to_string().contains(".."));
    }

    #[test]
    fn reject_github_component_double_dot() {
        assert!(validate_github_component("..", "org").is_err());
        assert!(validate_github_component(".", "org").is_err());
    }

    #[test]
    fn reject_github_component_leading_trailing_dot() {
        assert!(validate_github_component(".hidden", "org").is_err());
        assert!(validate_github_component("trailing.", "repo").is_err());
    }

    #[test]
    fn reject_git_ref_starting_with_slash() {
        assert!(validate_git_ref("/main").is_err());
    }

    #[test]
    fn reject_git_ref_ending_with_slash() {
        assert!(validate_git_ref("feature/").is_err());
    }

    #[test]
    fn split_ref_url_no_path() {
        let (url, git_ref) = split_ref("https://example.com");
        assert_eq!(url, "https://example.com");
        assert_eq!(git_ref, None);
    }

    #[test]
    fn reject_git_ref_leading_dash() {
        assert!(validate_git_ref("-evil").is_err());
        assert!(validate_git_ref("--double").is_err());
    }

    #[test]
    fn reject_git_ref_dot_lock_extension() {
        assert!(validate_git_ref("refs/heads/main.lock").is_err());
        assert!(validate_git_ref("branch.Lock").is_err());
    }

    #[test]
    fn plugin_id_hint_degenerate_all_special() {
        let src = GitSource::GitUrl {
            url: "https://example.com/___.git".to_string(),
            git_ref: None,
        };
        assert_eq!(src.plugin_id_hint(), "git-plugin");
    }

    #[test]
    fn plugin_id_hint_degenerate_empty_segment() {
        let src = GitSource::GitUrl {
            url: "https://example.com/.git".to_string(),
            git_ref: None,
        };
        assert_eq!(src.plugin_id_hint(), "git-plugin");
    }

    #[test]
    fn split_ref_multiple_at_in_path() {
        // Should split on the LAST @ in the path
        let (url, git_ref) = split_ref("https://host/p@th/repo@v1.0");
        assert_eq!(url, "https://host/p@th/repo");
        assert_eq!(git_ref, Some("v1.0".to_string()));
    }

    #[test]
    fn parse_bare_https_github() {
        let src = GitSource::parse("https://github.com/user/repo.git").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://github.com/user/repo.git".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_bare_https_github_no_dot_git() {
        let src = GitSource::parse("https://github.com/user/repo").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://github.com/user/repo".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_bare_https_github_with_ref() {
        let src = GitSource::parse("https://github.com/user/repo.git@main").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://github.com/user/repo.git".to_string(),
                git_ref: Some("main".to_string()),
            }
        );
    }

    #[test]
    fn parse_bare_https_gitlab() {
        let src = GitSource::parse("https://gitlab.com/org/repo").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://gitlab.com/org/repo".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_bare_https_dot_git_suffix() {
        let src = GitSource::parse("https://custom-host.com/org/repo.git").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://custom-host.com/org/repo.git".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_ssh_scp_style() {
        let src = GitSource::parse("git@github.com:user/repo.git").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "ssh://git@github.com/user/repo.git".to_string(),
                git_ref: None,
            }
        );
    }

    #[test]
    fn parse_ssh_scp_style_with_ref() {
        let src = GitSource::parse("git@github.com:user/repo.git@v2.0").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "ssh://git@github.com/user/repo.git".to_string(),
                git_ref: Some("v2.0".to_string()),
            }
        );
    }

    #[test]
    fn reject_bare_https_unknown_host() {
        // https:// without known host or .git suffix should not match
        assert!(GitSource::parse("https://example.com/something").is_err());
    }

    #[test]
    fn reject_spoofed_github_in_path() {
        // github.com in the *path* (not host) must NOT be accepted
        assert!(GitSource::parse("https://evil.com/github.com/payload").is_err());
    }

    #[test]
    fn reject_spoofed_gitlab_in_path() {
        assert!(GitSource::parse("https://evil.com/gitlab.com/payload").is_err());
    }

    #[test]
    fn parse_bare_https_custom_host_with_ref() {
        // .git extension with @ref on a non-github/gitlab host
        let src = GitSource::parse("https://custom-host.com/org/repo.git@main").unwrap();
        assert_eq!(
            src,
            GitSource::GitUrl {
                url: "https://custom-host.com/org/repo.git".to_string(),
                git_ref: Some("main".to_string()),
            }
        );
    }

    #[test]
    fn reject_ssh_path_traversal() {
        assert!(GitSource::parse("git@github.com:../../etc/passwd").is_err());
    }

    #[test]
    fn reject_ssh_host_with_spaces() {
        assert!(GitSource::parse("git@evil host:org/repo").is_err());
    }

    #[test]
    fn reject_ssh_host_with_slashes() {
        assert!(GitSource::parse("git@evil/host:org/repo").is_err());
    }

    #[test]
    fn reject_ssh_empty_host() {
        assert!(GitSource::parse("git@:org/repo").is_err());
    }

    #[test]
    fn reject_ssh_empty_path() {
        assert!(GitSource::parse("git@host:").is_err());
    }

    #[test]
    fn reject_ssh_no_colon() {
        assert!(GitSource::parse("git@host-no-colon").is_err());
    }
}
