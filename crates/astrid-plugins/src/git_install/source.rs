use super::validate::{
    validate_git_ref, validate_github_component, validate_ssh_host, validate_ssh_path,
    validate_url_scheme,
};
use crate::error::{PluginError, PluginResult};

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
/// Split a git repository URL from an optional `@ref` suffix.
///
/// Ensures we do not split on `@` symbols belonging to credentials (e.g. `ssh://git@host`).
#[must_use]
pub fn split_ref(s: &str) -> (String, Option<String>) {
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
