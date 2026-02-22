use super::source::GitSource;
use crate::error::{PluginError, PluginResult};
use std::path::PathBuf;
use tokio::process::Command;

const MAX_DOWNLOAD_SIZE: u64 = 100 * 1024 * 1024;

/// Timeout for git clone operations (5 minutes).
const GIT_CLONE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

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
            // Run async git clone with a timeout to prevent indefinite hangs.
            // When the timeout future drops, tokio::process::Command kills the child.
            let url = url.clone();
            let git_ref = git_ref.clone();
            tokio::time::timeout(GIT_CLONE_TIMEOUT, clone_git_repo(&url, git_ref.as_deref()))
                .await
                .map_err(|_| {
                    PluginError::ExecutionFailed(format!(
                        "git clone timed out after {}s",
                        GIT_CLONE_TIMEOUT.as_secs()
                    ))
                })?
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
        // Flat archives are rejected because stripping the first component would result
        // in an empty path and attempt to unpack a file over the dest directory itself.
        if entry_path.components().count() <= 1 && entry_type != tar::EntryType::Directory {
            return Err(PluginError::ExtractionError {
                message: format!(
                    "invalid archive format: expected root directory, found flat entry '{}'",
                    entry_path.display()
                ),
            });
        }
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
#[must_use]
pub fn strip_first_component(path: &std::path::Path) -> PathBuf {
    let mut components = path.components();
    components.next(); // skip first
    components.as_path().to_path_buf()
}

/// Clone a git repository into a temporary directory.
///
/// Suppresses interactive credential prompts and pipes stdin to null
/// to prevent hanging if authentication is required.
async fn clone_git_repo(
    url: &str,
    git_ref: Option<&str>,
) -> PluginResult<(tempfile::TempDir, PathBuf)> {
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
    cmd.kill_on_drop(true); // Ensure timeout cancellation kills the child process

    cmd.args(["clone", "--depth=1"]);

    if let Some(r) = git_ref {
        cmd.args(["--branch", r]);
    }

    cmd.arg(url);
    cmd.arg(&clone_path);

    let output = cmd
        .output()
        .await
        .map_err(|e| PluginError::ExecutionFailed(format!("failed to run git clone: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PluginError::ExecutionFailed(format!(
            "git clone failed:\n{stderr}"
        )));
    }

    Ok((tmp, clone_path))
}
