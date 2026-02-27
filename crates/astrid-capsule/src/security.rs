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

    /// Check whether the plugin is allowed to register a connector.
    ///
    /// Default implementation permits all registrations. Override to enforce
    /// connector policies (e.g. platform allowlists per plugin).
    ///
    /// RATIONALE: This has a permissive default (unlike the required file/HTTP
    /// methods) to maintain backward compatibility with existing
    /// `CapsuleSecurityGate` implementors. The `has_connector_capability` flag
    /// on `HostState` already gates access — this method adds operator-level
    /// policy on top.
    async fn check_connector_register(
        &self,
        _capsule_id: &str,
        _connector_name: &str,
        _platform: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

/// Security gate that permits all operations (for testing).
#[derive(Debug, Clone, Copy, Default)]
pub struct AllowAllGate;

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

    async fn check_connector_register(
        &self,
        _capsule_id: &str,
        _connector_name: &str,
        _platform: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

/// Security gate that denies all operations (for testing).
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyAllGate;

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

    async fn check_connector_register(
        &self,
        capsule_id: &str,
        connector_name: &str,
        platform: &str,
    ) -> Result<(), String> {
        Err(format!(
            "plugin '{capsule_id}' denied: register connector {connector_name} ({platform}) (DenyAllGate)"
        ))
    }
}

// ---------------------------------------------------------------------------
// Concrete adapter wrapping SecurityInterceptor (behind `approval` feature)
// ---------------------------------------------------------------------------

/// Security gate that enforces capabilities based on the manifest.
/// Assumes capabilities declared in the manifest were approved by the user during installation.
#[derive(Debug, Clone)]
pub struct ManifestSecurityGate {
    manifest: CapsuleManifest,
}

impl ManifestSecurityGate {
    pub fn new(manifest: CapsuleManifest) -> Self {
        Self { manifest }
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
        let path_obj = std::path::Path::new(path);
        if self
            .manifest
            .capabilities
            .fs_read
            .iter()
            .any(|p| p == "*" || path_obj.starts_with(p))
        {
            Ok(())
        } else {
            Err(format!(
                "plugin '{capsule_id}' denied: read access to '{path}' not declared in manifest"
            ))
        }
    }

    async fn check_file_write(&self, capsule_id: &str, path: &str) -> Result<(), String> {
        let path_obj = std::path::Path::new(path);
        if self
            .manifest
            .capabilities
            .fs_write
            .iter()
            .any(|p| p == "*" || path_obj.starts_with(p))
        {
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
}

#[cfg(feature = "approval")]
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
    pub struct SecurityInterceptorGate {
        interceptor: Arc<SecurityInterceptor>,
    }

    impl SecurityInterceptorGate {
        /// Wrap a `SecurityInterceptor` in this gate.
        #[must_use]
        pub fn new(interceptor: Arc<SecurityInterceptor>) -> Self {
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

        async fn check_connector_register(
            &self,
            capsule_id: &str,
            connector_name: &str,
            platform: &str,
        ) -> Result<(), String> {
            let action = SensitiveAction::CapsuleExecution {
                capsule_id: capsule_id.to_string(),
                capability: format!("register_connector({connector_name}, {platform})"),
            };
            self.interceptor
                .intercept(&action, "plugin host function: register connector", None)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
    }
}

#[cfg(feature = "approval")]
pub use interceptor_gate::SecurityInterceptorGate;

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
                kv: vec![],
                fs_read: fs_read.into_iter().map(String::from).collect(),
                fs_write: fs_write.into_iter().map(String::from).collect(),
                host_process: vec![],
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

    #[tokio::test]
    async fn test_manifest_security_gate_http() {
        let manifest = make_manifest(vec!["api.github.com"], vec![], vec![]);
        let gate = ManifestSecurityGate::new(manifest);

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
        let all_gate = ManifestSecurityGate::new(all_manifest);
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
        let gate = ManifestSecurityGate::new(manifest);

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

        // Write wildcard permits any path through the security gate.
        // Note: Actual VFS operations are still mathematically confined to the workspace root
        // by cap-std, so allowing it at the gate level is safe as long as the OS VFS is used.
        assert!(gate.check_file_write("test", "/etc/passwd").await.is_ok());
        assert!(
            gate.check_file_write("test", "/random/file.txt")
                .await
                .is_ok()
        );
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
        assert!(
            gate.check_connector_register("p", "my-conn", "discord")
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
        assert!(
            gate.check_connector_register("p", "my-conn", "discord")
                .await
                .is_err()
        );
    }
}
