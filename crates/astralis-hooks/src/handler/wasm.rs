//! WASM hook handler (stubbed for Phase 3).
//!
//! This module provides the interface for WASM-based hook handlers.
//! The actual implementation using Wasmtime will be added in Phase 3.

use std::time::Duration;
use tracing::warn;

use super::{HandlerError, HandlerResult};
use crate::hook::HookHandler;
use crate::result::{HookContext, HookExecutionResult};

/// Handler for WASM modules (stubbed).
///
/// This handler will execute WebAssembly modules using Wasmtime
/// in Phase 3. For now, it returns a stub response.
#[derive(Debug, Clone, Default)]
pub struct WasmHandler;

impl WasmHandler {
    /// Create a new WASM handler.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Execute a WASM handler (stubbed).
    ///
    /// # Errors
    ///
    /// Returns an error if the handler configuration is invalid.
    #[allow(clippy::unused_async)]
    pub async fn execute(
        &self,
        handler: &HookHandler,
        _context: &HookContext,
        _timeout: Duration,
    ) -> HandlerResult<HookExecutionResult> {
        let HookHandler::Wasm {
            module_path,
            function,
        } = handler
        else {
            return Err(HandlerError::InvalidConfiguration(
                "expected Wasm handler".to_string(),
            ));
        };

        warn!(
            module_path = %module_path,
            function = %function,
            "WASM handler is stubbed - will be implemented in Phase 3"
        );

        // For now, return a skipped result
        Ok(HookExecutionResult::Skipped {
            reason: format!(
                "WASM handlers are not yet implemented (module: {module_path}, function: {function})"
            ),
        })
    }

    /// Check if the WASM runtime is available.
    ///
    /// Always returns `false` until Phase 3 implementation.
    #[must_use]
    pub fn is_available() -> bool {
        false
    }
}

/// Configuration for WASM execution (for Phase 3).
#[derive(Debug, Clone)]
pub struct WasmConfig {
    /// Maximum memory in bytes.
    pub max_memory_bytes: u64,
    /// Maximum execution time.
    pub max_execution_time: Duration,
    /// Allowed host functions.
    pub allowed_imports: Vec<String>,
    /// Enable WASI.
    pub enable_wasi: bool,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            max_memory_bytes: 64 * 1024 * 1024, // 64 MB
            max_execution_time: Duration::from_secs(30),
            allowed_imports: vec![
                "astralis_log".to_string(),
                "astralis_get_context".to_string(),
                "astralis_set_result".to_string(),
            ],
            enable_wasi: true,
        }
    }
}

/// WASM module metadata (for Phase 3).
#[derive(Debug, Clone)]
pub struct WasmModuleInfo {
    /// Module path.
    pub path: String,
    /// Module hash (for verification).
    pub hash: Option<String>,
    /// Exported functions.
    pub exports: Vec<String>,
    /// Required imports.
    pub imports: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::HookEvent;

    #[tokio::test]
    async fn test_wasm_handler_stubbed() {
        let handler = WasmHandler::new();
        let hook_handler = HookHandler::Wasm {
            module_path: "/path/to/module.wasm".to_string(),
            function: "handle".to_string(),
        };
        let context = HookContext::new(HookEvent::PreToolCall);

        let result = handler
            .execute(&hook_handler, &context, Duration::from_secs(5))
            .await
            .unwrap();

        assert!(matches!(result, HookExecutionResult::Skipped { .. }));
    }

    #[test]
    fn test_wasm_not_available() {
        assert!(!WasmHandler::is_available());
    }

    #[test]
    fn test_wasm_config_default() {
        let config = WasmConfig::default();
        assert_eq!(config.max_memory_bytes, 64 * 1024 * 1024);
        assert!(config.enable_wasi);
    }
}
