//! Security gate trait for plugin host function calls.
//!
//! Decouples the plugin WASM runtime from the full security interceptor stack.
//! Test implementations ([`AllowAllGate`], [`DenyAllGate`]) are provided for
//! unit testing. A concrete [`SecurityInterceptorGate`] adapter wrapping
//! `astrid-approval`'s `SecurityInterceptor` is available behind the
//! `approval` feature flag.

use crate::manifest::CapsuleManifest;
use async_trait::async_trait;

/// Security gate for plugin host function calls.
///
/// Each method corresponds to a class of sensitive operation that a WASM
/// plugin can request through host functions. Implementors decide whether
/// to permit or deny the operation.
#[async_trait]
pub trait CapsuleSecurityGate: Send + Sync {
    /// Check whether the plugin is allowed to make an HTTP request.
    async fn check_http_request(
        &self,
        capsule_id: &str,
        method: &str,
        url: &str,
    ) -> Result<(), String>;

    /// Check whether the plugin is allowed to read a file.
    async fn check_file_read(&self, capsule_id: &str, path: &str) -> Result<(), String>;

    /// Check whether the plugin is allowed to write a file.
    async fn check_file_write(&self, capsule_id: &str, path: &str) -> Result<(), String>;

    /// Check whether the plugin is allowed to spawn a host process.
    async fn check_host_process(&self, capsule_id: &str, command: &str) -> Result<(), String>;

    /// Check whether the plugin is allowed to accept connections on a bound socket.
    ///
    /// Default implementation denies all bind operations. Override to permit
    /// capsules that declare `net_bind` capabilities.
    ///
    /// NOTE: This method currently takes no socket path argument because the
    /// kernel pre-binds the socket and the path is not user-controllable.
    /// If future work introduces capsule-specified bind addresses, add a
    /// `socket_path: &str` parameter and enforce path-based confinement.
    async fn check_net_bind(&self, capsule_id: &str) -> Result<(), String> {
        Err(format!(
            "plugin '{capsule_id}' denied: net_bind not permitted (default)"
        ))
    }

    /// Check whether the plugin is allowed to register a uplink.
    ///
    /// Default implementation permits all registrations. Override to enforce
    /// uplink policies (e.g. platform allowlists per plugin).
    ///
    /// RATIONALE: This has a permissive default (unlike the required file/HTTP
    /// methods) to maintain backward compatibility with existing
    /// `CapsuleSecurityGate` implementors. The `has_uplink_capability` flag
    /// on `HostState` already gates access — this method adds operator-level
    /// policy on top.
    async fn check_uplink_register(
        &self,
        _capsule_id: &str,
        _uplink_name: &str,
        _platform: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

/// Security gate that permits all operations (for testing).
#[derive(Debug, Clone, Copy, Default)]
#[cfg(test)]
pub(crate) struct AllowAllGate;

#[cfg(test)]
#[async_trait]
impl CapsuleSecurityGate for AllowAllGate {
    async fn check_http_request(
        &self,
        _capsule_id: &str,
        _method: &str,
        _url: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn check_file_read(&self, _capsule_id: &str, _path: &str) -> Result<(), String> {
        Ok(())
    }

    async fn check_file_write(&self, _capsule_id: &str, _path: &str) -> Result<(), String> {
        Ok(())
    }

    async fn check_host_process(&self, _capsule_id: &str, _command: &str) -> Result<(), String> {
        Ok(())
    }

    async fn check_net_bind(&self, _capsule_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn check_uplink_register(
        &self,
        _capsule_id: &str,
        _uplink_name: &str,
        _platform: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

/// Security gate that denies all operations (for testing).
#[cfg(test)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DenyAllGate;

#[cfg(test)]
#[async_trait]
impl CapsuleSecurityGate for DenyAllGate {
    async fn check_http_request(
        &self,
        capsule_id: &str,
        method: &str,
        url: &str,
    ) -> Result<(), String> {
        Err(format!(
            "plugin '{capsule_id}' denied: {method} {url} (DenyAllGate)"
        ))
    }

    async fn check_file_read(&self, capsule_id: &str, path: &str) -> Result<(), String> {
        Err(format!(
            "plugin '{capsule_id}' denied: read {path} (DenyAllGate)"
        ))
    }

    async fn check_file_write(&self, capsule_id: &str, path: &str) -> Result<(), String> {
        Err(format!(
            "plugin '{capsule_id}' denied: write {path} (DenyAllGate)"
        ))
    }

    async fn check_host_process(&self, capsule_id: &str, command: &str) -> Result<(), String> {
        Err(format!(
            "plugin '{capsule_id}' denied: spawn host process {command} (DenyAllGate)"
        ))
    }

    async fn check_net_bind(&self, capsule_id: &str) -> Result<(), String> {
        Err(format!(
            "plugin '{capsule_id}' denied: net_bind (DenyAllGate)"
        ))
    }

    async fn check_uplink_register(
        &self,
        capsule_id: &str,
        uplink_name: &str,
        platform: &str,
    ) -> Result<(), String> {
        Err(format!(
            "plugin '{capsule_id}' denied: register uplink {uplink_name} ({platform}) (DenyAllGate)"
        ))
    }
}

// ---------------------------------------------------------------------------
// Concrete adapter wrapping SecurityInterceptor (behind `approval` feature)
// ---------------------------------------------------------------------------

/// Security gate that enforces capabilities based on the manifest.
/// Assumes capabilities declared in the manifest were approved by the user during installation.
///
/// VFS scheme prefixes (`workspace://`, `global://`) in `fs_read` / `fs_write`
/// capability entries are resolved to their physical root paths at construction
/// time so that runtime path checks use simple `starts_with` matching.
#[derive(Debug, Clone)]
pub(crate) struct ManifestSecurityGate {
    /// The original manifest. `net` and `host_process` fields are queried
    /// at runtime as-is. `fs_read` / `fs_write` are **not** used at runtime —
    /// their scheme-resolved equivalents (`resolved_fs_read` / `resolved_fs_write`)
    /// are used instead. If you add a new scheme-aware capability field, add a
    /// corresponding `resolved_*` field and resolve it in `new()`.
    manifest: CapsuleManifest,
    /// Resolved filesystem prefixes for read access (scheme prefixes expanded
    /// to canonical physical paths at construction time).
    resolved_fs_read: Vec<String>,
    /// Resolved filesystem prefixes for write access (scheme prefixes expanded
    /// to canonical physical paths at construction time).
    resolved_fs_write: Vec<String>,
    /// Canonical workspace root used to confine wildcard (`"*"`) file access.
    /// Wildcard only matches paths under this root — not the entire filesystem.
    /// Stored as `PathBuf` so that `Path::starts_with` handles component-boundary
    /// matching correctly (e.g. `/workspace-evil` does NOT match `/workspace`).
    workspace_root_path: std::path::PathBuf,
}

impl ManifestSecurityGate {
    pub(crate) fn new(
        manifest: CapsuleManifest,
        workspace_root: std::path::PathBuf,
        global_root: Option<std::path::PathBuf>,
    ) -> Self {
        // Canonicalize roots once up front. Both `resolve_schemes` (for prefix
        // strings) and `workspace_root_path` (for wildcard confinement) use
        // the same canonical values, avoiding redundant syscalls.
        let canonical_ws = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf());
        let canonical_global = global_root
            .as_ref()
            .map(|g| g.canonicalize().unwrap_or_else(|_| g.clone()));

        let resolved_fs_read = Self::resolve_schemes(
            &manifest.capabilities.fs_read,
            &canonical_ws,
            &canonical_global,
        );
        let resolved_fs_write = Self::resolve_schemes(
            &manifest.capabilities.fs_write,
            &canonical_ws,
            &canonical_global,
        );
        Self {
            manifest,
            resolved_fs_read,
            resolved_fs_write,
            workspace_root_path: canonical_ws,
        }
    }

    /// Translate VFS scheme prefixes into physical paths.
    ///
    /// - `workspace://` -> `<workspace_root>/`
    /// - `global://` -> `<global_root>/` (dropped if no global root is configured)
    /// - `*` -> kept as-is (wildcard — confined at check time)
    /// - anything else -> kept as-is (literal path prefix for backwards compat)
    ///
    /// Expects pre-canonicalized roots (canonicalization is done once in `new()`).
    fn resolve_schemes(
        entries: &[String],
        canonical_ws: &std::path::Path,
        canonical_global: &Option<std::path::PathBuf>,
    ) -> Vec<String> {
        let mut resolved = Vec::with_capacity(entries.len());
        for entry in entries {
            if entry == "*" {
                resolved.push("*".to_string());
            } else if let Some(suffix) = entry.strip_prefix("workspace://") {
                let path = canonical_ws.join(suffix);
                resolved.push(path.to_string_lossy().to_string());
            } else if let Some(suffix) = entry.strip_prefix("global://") {
                if let Some(g_root) = canonical_global {
                    let path = g_root.join(suffix);
                    resolved.push(path.to_string_lossy().to_string());
                }
                // If no global root is configured, silently drop this entry
                // so the capsule simply cannot access global paths.
            } else {
                resolved.push(entry.clone());
            }
        }
        resolved
    }

    /// Check a filesystem path against a list of resolved allowed patterns.
    ///
    /// Rejects paths containing `..` (ParentDir) components to prevent traversal
    /// attacks like `/workspace/../../etc/passwd` which would pass a naive
    /// `starts_with` check. Uses `Path::starts_with` for component-boundary
    /// matching, so `/workspace-evil` does NOT match `/workspace`.
    ///
    /// When a wildcard `"*"` is present, it only matches paths under the
    /// canonical workspace root — preventing escape to global paths
    /// (e.g. `~/.astrid/keys/`).
    fn check_fs_permission(&self, path: &str, resolved: &[String]) -> bool {
        let path_obj = std::path::Path::new(path);

        // Reject paths with '..' components — these can bypass starts_with checks
        // (e.g. /workspace/../../etc/passwd starts_with /workspace but resolves outside).
        if path_obj
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return false;
        }

        resolved.iter().any(|p| {
            if p == "*" {
                path_obj.starts_with(&self.workspace_root_path)
            } else {
                path_obj.starts_with(p)
            }
        })
    }
}

#[async_trait]
impl CapsuleSecurityGate for ManifestSecurityGate {
    async fn check_http_request(
        &self,
        capsule_id: &str,
        _method: &str,
        url: &str,
    ) -> Result<(), String> {
        let parsed_url = reqwest::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;
        let host_str = parsed_url.host_str().unwrap_or("");

        if self
            .manifest
            .capabilities
            .net
            .iter()
            .any(|d| d == "*" || host_str == d || host_str.ends_with(&format!(".{d}")))
        {
            Ok(())
        } else {
            Err(format!(
                "plugin '{capsule_id}' denied: network access to '{url}' not declared in manifest"
            ))
        }
    }

    async fn check_file_read(&self, capsule_id: &str, path: &str) -> Result<(), String> {
        if self.check_fs_permission(path, &self.resolved_fs_read) {
            Ok(())
        } else {
            Err(format!(
                "plugin '{capsule_id}' denied: read access to '{path}' not declared in manifest"
            ))
        }
    }

    async fn check_file_write(&self, capsule_id: &str, path: &str) -> Result<(), String> {
        if self.check_fs_permission(path, &self.resolved_fs_write) {
            Ok(())
        } else {
            Err(format!(
                "plugin '{capsule_id}' denied: write access to '{path}' not declared in manifest"
            ))
        }
    }

    async fn check_host_process(&self, capsule_id: &str, command: &str) -> Result<(), String> {
        if self
            .manifest
            .capabilities
            .host_process
            .iter()
            .any(|cmd| command == cmd || command.starts_with(&format!("{cmd} ")))
        {
            Ok(())
        } else {
            Err(format!(
                "plugin '{capsule_id}' denied: host process '{command}' not declared in manifest"
            ))
        }
    }

    async fn check_net_bind(&self, capsule_id: &str) -> Result<(), String> {
        // Require at least one non-empty net_bind entry. Empty strings in the
        // manifest are treated as malformed and do not grant capability.
        let has_valid_entry = self
            .manifest
            .capabilities
            .net_bind
            .iter()
            .any(|entry| !entry.is_empty());
        if has_valid_entry {
            Ok(())
        } else {
            Err(format!(
                "plugin '{capsule_id}' denied: net_bind not declared in manifest"
            ))
        }
    }
}

#[cfg(feature = "approval")]
#[allow(dead_code)] // Tracked by #302
mod interceptor_gate {
    use super::{CapsuleSecurityGate, async_trait};
    use astrid_approval::action::SensitiveAction;
    use astrid_approval::interceptor::SecurityInterceptor;
    use astrid_core::types::Permission;
    use std::sync::Arc;

    /// Adapter that delegates security checks to [`SecurityInterceptor`].
    ///
    /// Creates the appropriate [`SensitiveAction`] variant for each operation
    /// and calls `interceptor.intercept()`. A successful intercept means the
    /// operation is allowed; an error means it is denied.
    pub(super) struct SecurityInterceptorGate {
        interceptor: Arc<SecurityInterceptor>,
    }

    impl SecurityInterceptorGate {
        /// Wrap a `SecurityInterceptor` in this gate.
        #[must_use]
        pub(super) fn new(interceptor: Arc<SecurityInterceptor>) -> Self {
            Self { interceptor }
        }
    }

    impl std::fmt::Debug for SecurityInterceptorGate {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SecurityInterceptorGate")
                .finish_non_exhaustive()
        }
    }

    #[async_trait]
    impl CapsuleSecurityGate for SecurityInterceptorGate {
        async fn check_http_request(
            &self,
            capsule_id: &str,
            method: &str,
            url: &str,
        ) -> Result<(), String> {
            let action = SensitiveAction::CapsuleHttpRequest {
                capsule_id: capsule_id.to_string(),
                url: url.to_string(),
                method: method.to_string(),
            };
            self.interceptor
                .intercept(&action, "plugin host function: HTTP request", None)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        async fn check_file_read(&self, capsule_id: &str, path: &str) -> Result<(), String> {
            let action = SensitiveAction::CapsuleFileAccess {
                capsule_id: capsule_id.to_string(),
                path: path.to_string(),
                mode: Permission::Read,
            };
            self.interceptor
                .intercept(&action, "plugin host function: file read", None)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        async fn check_file_write(&self, capsule_id: &str, path: &str) -> Result<(), String> {
            let action = SensitiveAction::CapsuleFileAccess {
                capsule_id: capsule_id.to_string(),
                path: path.to_string(),
                mode: Permission::Write,
            };
            self.interceptor
                .intercept(&action, "plugin host function: file write", None)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        async fn check_host_process(&self, capsule_id: &str, command: &str) -> Result<(), String> {
            let action = SensitiveAction::CapsuleExecution {
                capsule_id: capsule_id.to_string(),
                capability: format!("host_process: {command}"),
            };
            self.interceptor
                .intercept(&action, "plugin host function: host process", None)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        async fn check_net_bind(&self, capsule_id: &str) -> Result<(), String> {
            let action = SensitiveAction::CapsuleNetBind {
                capsule_id: capsule_id.to_string(),
            };
            self.interceptor
                .intercept(&action, "plugin host function: net_bind accept", None)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        async fn check_uplink_register(
            &self,
            capsule_id: &str,
            uplink_name: &str,
            platform: &str,
        ) -> Result<(), String> {
            let action = SensitiveAction::CapsuleExecution {
                capsule_id: capsule_id.to_string(),
                capability: format!("register_uplink({uplink_name}, {platform})"),
            };
            self.interceptor
                .intercept(&action, "plugin host function: register uplink", None)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{CapabilitiesDef, CapsuleManifest, PackageDef};

    fn make_manifest(net: Vec<&str>, fs_read: Vec<&str>, fs_write: Vec<&str>) -> CapsuleManifest {
        CapsuleManifest {
            package: PackageDef {
                name: "test".into(),
                version: "0.1.0".into(),
                description: None,
                authors: vec![],
                repository: None,
                homepage: None,
                documentation: None,
                license: None,
                license_file: None,
                readme: None,
                keywords: vec![],
                categories: vec![],
                astrid_version: None,
                publish: None,
                include: None,
                exclude: None,
                metadata: None,
            },
            components: vec![],
            dependencies: Default::default(),
            capabilities: CapabilitiesDef {
                net: net.into_iter().map(String::from).collect(),
                net_bind: vec![],
                kv: vec![],
                fs_read: fs_read.into_iter().map(String::from).collect(),
                fs_write: fs_write.into_iter().map(String::from).collect(),
                host_process: vec![],
                uplink: false,
                ipc_publish: vec![],
            },
            env: Default::default(),
            context_files: vec![],
            commands: vec![],
            mcp_servers: vec![],
            skills: vec![],
            uplinks: vec![],
            llm_providers: vec![],
            interceptors: vec![],
            cron_jobs: vec![],
            tools: vec![],
        }
    }

    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from("/workspace")
    }

    fn global_root() -> std::path::PathBuf {
        std::path::PathBuf::from("/home/user/.astrid")
    }

    #[tokio::test]
    async fn test_manifest_security_gate_http() {
        let manifest = make_manifest(vec!["api.github.com"], vec![], vec![]);
        let gate = ManifestSecurityGate::new(manifest, workspace_root(), None);

        assert!(
            gate.check_http_request("test", "GET", "https://api.github.com/v1")
                .await
                .is_ok()
        );
        assert!(
            gate.check_http_request("test", "GET", "https://v1.api.github.com/v1")
                .await
                .is_ok()
        );
        assert!(
            gate.check_http_request("test", "GET", "https://evil.com/v1")
                .await
                .is_err()
        );
        assert!(
            gate.check_http_request("test", "GET", "http://api.github.com@127.0.0.1/admin")
                .await
                .is_err()
        );
        assert!(
            gate.check_http_request("test", "GET", "http://github.com/v1")
                .await
                .is_err()
        );

        let all_manifest = make_manifest(vec!["*"], vec![], vec![]);
        let all_gate = ManifestSecurityGate::new(all_manifest, workspace_root(), None);
        assert!(
            all_gate
                .check_http_request("test", "GET", "https://evil.com/v1")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_manifest_security_gate_fs() {
        let manifest = make_manifest(vec![], vec!["/workspace/src", "/tmp/exact.txt"], vec!["*"]);
        let gate = ManifestSecurityGate::new(manifest, workspace_root(), None);

        // Path matches correctly
        assert!(
            gate.check_file_read("test", "/workspace/src/main.rs")
                .await
                .is_ok()
        );
        assert!(gate.check_file_read("test", "/tmp/exact.txt").await.is_ok());

        // Path boundary correctly enforced
        assert!(
            gate.check_file_read("test", "/workspace/src-evil/main.rs")
                .await
                .is_err()
        );
        assert!(
            gate.check_file_read("test", "/workspace/src_evil/main.rs")
                .await
                .is_err()
        );
        assert!(gate.check_file_read("test", "/workspace/src").await.is_ok()); // Exact match is OK

        // Write wildcard is confined to workspace root — paths outside are denied.
        assert!(
            gate.check_file_write("test", "/workspace/src/main.rs")
                .await
                .is_ok()
        );
        assert!(gate.check_file_write("test", "/etc/passwd").await.is_err());
        assert!(
            gate.check_file_write("test", "/random/file.txt")
                .await
                .is_err()
        );

        // Path traversal via .. must be rejected even with explicit prefix match
        assert!(
            gate.check_file_read("test", "/workspace/src/../../etc/passwd")
                .await
                .is_err(),
            "path traversal via .. must be rejected"
        );
    }

    #[tokio::test]
    async fn test_scheme_resolution_workspace() {
        let manifest = make_manifest(vec![], vec!["workspace://"], vec![]);
        let gate = ManifestSecurityGate::new(manifest, workspace_root(), None);

        assert!(
            gate.check_file_read("test", "/workspace/src/main.rs")
                .await
                .is_ok()
        );
        assert!(gate.check_file_read("test", "/other/path").await.is_err());
    }

    #[tokio::test]
    async fn test_scheme_resolution_global() {
        let manifest = make_manifest(vec![], vec!["global://"], vec![]);
        let gate = ManifestSecurityGate::new(manifest, workspace_root(), Some(global_root()));

        assert!(
            gate.check_file_read("test", "/home/user/.astrid/skills/my-skill/SKILL.md")
                .await
                .is_ok()
        );
        assert!(
            gate.check_file_read("test", "/workspace/src/main.rs")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_scheme_resolution_global_without_root() {
        // When no global root is configured, global:// entries are silently dropped
        let manifest = make_manifest(vec![], vec!["global://"], vec![]);
        let gate = ManifestSecurityGate::new(manifest, workspace_root(), None);

        assert!(
            gate.check_file_read("test", "/home/user/.astrid/skills/my-skill/SKILL.md")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_scheme_resolution_both() {
        let manifest = make_manifest(vec![], vec!["workspace://", "global://"], vec![]);
        let gate = ManifestSecurityGate::new(manifest, workspace_root(), Some(global_root()));

        assert!(
            gate.check_file_read("test", "/workspace/src/main.rs")
                .await
                .is_ok()
        );
        assert!(
            gate.check_file_read("test", "/home/user/.astrid/config.toml")
                .await
                .is_ok()
        );
        assert!(gate.check_file_read("test", "/etc/passwd").await.is_err());
    }

    #[tokio::test]
    async fn test_global_path_denied_without_manifest_entry() {
        // Manifest only has workspace://, no global:// — global paths must be denied
        // even when global_root is configured.
        let manifest = make_manifest(vec![], vec!["workspace://"], vec![]);
        let gate = ManifestSecurityGate::new(manifest, workspace_root(), Some(global_root()));

        assert!(
            gate.check_file_read("test", "/home/user/.astrid/skills/foo/SKILL.md")
                .await
                .is_err()
        );
        // Workspace paths should still work
        assert!(
            gate.check_file_read("test", "/workspace/src/main.rs")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn wildcard_confined_to_workspace_root() {
        // Use a real tempdir so canonicalize() resolves correctly on all platforms
        // (e.g. macOS /tmp -> /private/tmp).
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("project");
        std::fs::create_dir_all(&ws).unwrap();
        let canonical_ws = ws.canonicalize().unwrap();

        let manifest = make_manifest(vec![], vec!["*"], vec!["*"]);
        let gate = ManifestSecurityGate::new(manifest, ws, None);

        // Paths under the canonical workspace root are allowed
        let read_path = canonical_ws.join("src/main.rs");
        assert!(
            gate.check_file_read("test", read_path.to_str().unwrap())
                .await
                .is_ok()
        );
        let write_path = canonical_ws.join("out/file.txt");
        assert!(
            gate.check_file_write("test", write_path.to_str().unwrap())
                .await
                .is_ok()
        );

        // Paths outside the workspace root are denied even with wildcard
        assert!(gate.check_file_read("test", "/etc/passwd").await.is_err());
        assert!(
            gate.check_file_write("test", "/home/user/.astrid/keys/user.key")
                .await
                .is_err()
        );

        // Prefix-collision attack: /project-evil should NOT match /project
        let evil_path = canonical_ws.parent().unwrap().join("project-evil/file.txt");
        assert!(
            gate.check_file_write("test", evil_path.to_str().unwrap())
                .await
                .is_err()
        );

        // Path traversal attack: /workspace/../../etc/passwd must be rejected
        // even though it starts_with /workspace at component level.
        let traversal = format!("{}/../../etc/passwd", canonical_ws.display());
        assert!(
            gate.check_file_read("test", &traversal).await.is_err(),
            "path traversal via .. must be rejected"
        );
        assert!(
            gate.check_file_write("test", &traversal).await.is_err(),
            "path traversal via .. must be rejected for writes"
        );
    }

    #[tokio::test]
    async fn net_bind_gate_enforced() {
        // No net_bind capability -> denied
        let manifest = make_manifest(vec![], vec![], vec![]);
        let gate = ManifestSecurityGate::new(manifest, workspace_root(), None);
        assert!(gate.check_net_bind("test").await.is_err());

        // With net_bind capability -> allowed
        let mut manifest2 = make_manifest(vec![], vec![], vec![]);
        manifest2.capabilities.net_bind = vec!["unix:///tmp/sock".into()];
        let gate2 = ManifestSecurityGate::new(manifest2, workspace_root(), None);
        assert!(gate2.check_net_bind("test").await.is_ok());

        // Empty string in net_bind is treated as malformed -> denied
        let mut manifest3 = make_manifest(vec![], vec![], vec![]);
        manifest3.capabilities.net_bind = vec!["".into()];
        let gate3 = ManifestSecurityGate::new(manifest3, workspace_root(), None);
        assert!(gate3.check_net_bind("test").await.is_err());
    }

    #[tokio::test]
    async fn allow_all_gate_permits_everything() {
        let gate = AllowAllGate;
        assert!(
            gate.check_http_request("p", "GET", "http://x")
                .await
                .is_ok()
        );
        assert!(gate.check_file_read("p", "/tmp/f").await.is_ok());
        assert!(gate.check_file_write("p", "/tmp/f").await.is_ok());
        assert!(gate.check_net_bind("p").await.is_ok());
        assert!(
            gate.check_uplink_register("p", "my-conn", "discord")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn deny_all_gate_rejects_everything() {
        let gate = DenyAllGate;
        assert!(
            gate.check_http_request("p", "GET", "http://x")
                .await
                .is_err()
        );
        assert!(gate.check_file_read("p", "/tmp/f").await.is_err());
        assert!(gate.check_file_write("p", "/tmp/f").await.is_err());
        assert!(gate.check_net_bind("p").await.is_err());
        assert!(
            gate.check_uplink_register("p", "my-conn", "discord")
                .await
                .is_err()
        );
    }
}
