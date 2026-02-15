//! Security gate trait for plugin host function calls.
//!
//! Decouples the plugin WASM runtime from the full security interceptor stack.
//! Test implementations ([`AllowAllGate`], [`DenyAllGate`]) are provided for
//! unit testing. A concrete [`SecurityInterceptorGate`] adapter wrapping
//! `astrid-approval`'s `SecurityInterceptor` is available behind the
//! `approval` feature flag.

use async_trait::async_trait;

/// Security gate for plugin host function calls.
///
/// Each method corresponds to a class of sensitive operation that a WASM
/// plugin can request through host functions. Implementors decide whether
/// to permit or deny the operation.
#[async_trait]
pub trait PluginSecurityGate: Send + Sync {
    /// Check whether the plugin is allowed to make an HTTP request.
    async fn check_http_request(
        &self,
        plugin_id: &str,
        method: &str,
        url: &str,
    ) -> Result<(), String>;

    /// Check whether the plugin is allowed to read a file.
    async fn check_file_read(&self, plugin_id: &str, path: &str) -> Result<(), String>;

    /// Check whether the plugin is allowed to write a file.
    async fn check_file_write(&self, plugin_id: &str, path: &str) -> Result<(), String>;
}

/// Security gate that permits all operations (for testing).
#[derive(Debug, Clone, Copy, Default)]
pub struct AllowAllGate;

#[async_trait]
impl PluginSecurityGate for AllowAllGate {
    async fn check_http_request(
        &self,
        _plugin_id: &str,
        _method: &str,
        _url: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn check_file_read(&self, _plugin_id: &str, _path: &str) -> Result<(), String> {
        Ok(())
    }

    async fn check_file_write(&self, _plugin_id: &str, _path: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Security gate that denies all operations (for testing).
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyAllGate;

#[async_trait]
impl PluginSecurityGate for DenyAllGate {
    async fn check_http_request(
        &self,
        plugin_id: &str,
        method: &str,
        url: &str,
    ) -> Result<(), String> {
        Err(format!(
            "plugin '{plugin_id}' denied: {method} {url} (DenyAllGate)"
        ))
    }

    async fn check_file_read(&self, plugin_id: &str, path: &str) -> Result<(), String> {
        Err(format!(
            "plugin '{plugin_id}' denied: read {path} (DenyAllGate)"
        ))
    }

    async fn check_file_write(&self, plugin_id: &str, path: &str) -> Result<(), String> {
        Err(format!(
            "plugin '{plugin_id}' denied: write {path} (DenyAllGate)"
        ))
    }
}

// ---------------------------------------------------------------------------
// Concrete adapter wrapping SecurityInterceptor (behind `approval` feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "approval")]
mod interceptor_gate {
    use super::{PluginSecurityGate, async_trait};
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
    impl PluginSecurityGate for SecurityInterceptorGate {
        async fn check_http_request(
            &self,
            plugin_id: &str,
            method: &str,
            url: &str,
        ) -> Result<(), String> {
            let action = SensitiveAction::PluginHttpRequest {
                plugin_id: plugin_id.to_string(),
                url: url.to_string(),
                method: method.to_string(),
            };
            self.interceptor
                .intercept(&action, "plugin host function: HTTP request", None)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        async fn check_file_read(&self, plugin_id: &str, path: &str) -> Result<(), String> {
            let action = SensitiveAction::PluginFileAccess {
                plugin_id: plugin_id.to_string(),
                path: path.to_string(),
                mode: Permission::Read,
            };
            self.interceptor
                .intercept(&action, "plugin host function: file read", None)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        async fn check_file_write(&self, plugin_id: &str, path: &str) -> Result<(), String> {
            let action = SensitiveAction::PluginFileAccess {
                plugin_id: plugin_id.to_string(),
                path: path.to_string(),
                mode: Permission::Write,
            };
            self.interceptor
                .intercept(&action, "plugin host function: file write", None)
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
    }
}
