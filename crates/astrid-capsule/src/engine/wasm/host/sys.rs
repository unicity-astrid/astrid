use crate::engine::wasm::bindings::astrid::capsule::sys;
use crate::engine::wasm::bindings::astrid::capsule::types::{
    CallerContext, CapabilityCheckRequest, CapabilityCheckResponse, LogLevel,
};
use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

/// Trigger request sent by WASM capsules via `hooks::trigger`.
#[derive(serde::Deserialize)]
struct TriggerRequest {
    /// The hook/interceptor topic to fan out (e.g. `before_tool_call`).
    hook: String,
    /// Opaque JSON payload forwarded to each matching interceptor.
    payload: serde_json::Value,
}

impl sys::Host for HostState {
    fn get_config(&mut self, key: String) -> Result<String, String> {
        let value = self.config.get(&key).cloned();

        // Return the raw string value, not JSON-encoded.
        // serde_json::to_string wraps strings in quotes ("\"value\""),
        // causing double-encoding when the SDK's env::var reads it.
        let result = match value {
            Some(serde_json::Value::String(s)) => s,
            Some(v) => serde_json::to_string(&v).unwrap_or_default(),
            None => String::new(),
        };
        Ok(result)
    }

    fn get_caller(&mut self) -> Result<CallerContext, String> {
        if let Some(ref msg) = self.caller_context {
            Ok(CallerContext {
                principal: msg.principal.clone(),
                source_id: msg.source_id.to_string(),
                timestamp: msg.timestamp.to_rfc3339(),
            })
        } else {
            Ok(CallerContext {
                principal: None,
                source_id: String::new(),
                timestamp: String::new(),
            })
        }
    }

    fn trigger_hook(&mut self, request_json: String) -> Result<String, String> {
        let caller_id = self.capsule_id.clone();
        let registry = self.capsule_registry.clone();
        let rt_handle = self.runtime_handle.clone();
        let host_semaphore = self.host_semaphore.clone();

        let result_str = if let Some(registry) = registry {
            // Deserialize the trigger request from the WASM guest.
            let request: TriggerRequest = serde_json::from_str(&request_json)
                .map_err(|e| format!("invalid trigger request: {e}"))?;

            let payload_bytes = serde_json::to_vec(&request.payload).unwrap_or_default();

            // Fan out: find all capsules with interceptors matching the hook topic,
            // invoke each (skipping the caller to prevent infinite recursion),
            // and collect their responses.
            //
            // Step 1: Collect matching capsules under the registry read lock.
            let matches: Vec<(std::sync::Arc<dyn crate::capsule::Capsule>, String)> =
                util::bounded_block_on(&rt_handle, &host_semaphore, async {
                    let registry = registry.read().await;
                    let mut matches = Vec::new();

                    for capsule_id in registry.list() {
                        // Skip the calling capsule to prevent recursion.
                        if *capsule_id == caller_id {
                            continue;
                        }
                        if let Some(capsule) = registry.get(capsule_id) {
                            if !matches!(capsule.state(), crate::capsule::CapsuleState::Ready) {
                                continue;
                            }
                            for interceptor in &capsule.manifest().interceptors {
                                if crate::topic::topic_matches(&request.hook, &interceptor.event) {
                                    matches.push((
                                        std::sync::Arc::clone(&capsule),
                                        interceptor.action.clone(),
                                    ));
                                }
                            }
                        }
                    }
                    matches
                });

            // Step 2: Dispatch each interceptor via spawned tasks and collect
            // results.
            let responses: Vec<serde_json::Value> =
                util::bounded_block_on(&rt_handle, &host_semaphore, async {
                    let mut join_set = tokio::task::JoinSet::new();

                    for (capsule, action) in matches {
                        let payload = payload_bytes.clone();
                        let hook = request.hook.clone();
                        join_set.spawn(async move {
                            match capsule.invoke_interceptor(&action, &payload, None) {
                                Ok(crate::capsule::InterceptResult::Continue(bytes))
                                    if bytes.is_empty() =>
                                {
                                    None
                                },
                                Ok(
                                    crate::capsule::InterceptResult::Continue(bytes)
                                    | crate::capsule::InterceptResult::Final(bytes),
                                ) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                                    Ok(val) => Some(val),
                                    Err(_) => {
                                        tracing::warn!(
                                            capsule_id = %capsule.id(),
                                            action = %action,
                                            "interceptor returned non-JSON response, skipping"
                                        );
                                        None
                                    },
                                },
                                Ok(crate::capsule::InterceptResult::Deny { reason }) => {
                                    tracing::warn!(
                                        capsule_id = %capsule.id(),
                                        action = %action,
                                        hook = %hook,
                                        reason = %reason,
                                        "interceptor denied during hook trigger"
                                    );
                                    None
                                },
                                Err(e) => {
                                    tracing::warn!(
                                        capsule_id = %capsule.id(),
                                        action = %action,
                                        hook = %hook,
                                        error = %e,
                                        "interceptor invocation failed during hook trigger"
                                    );
                                    None
                                },
                            }
                        });
                    }

                    let mut responses = Vec::new();
                    while let Some(result) = join_set.join_next().await {
                        if let Ok(Some(val)) = result {
                            responses.push(val);
                        }
                    }
                    responses
                });

            match serde_json::to_string(&responses) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to serialize hook responses");
                    "[]".to_string()
                },
            }
        } else {
            // No registry available — return empty array (no subscribers).
            "[]".to_string()
        };

        Ok(result_str)
    }

    fn log(&mut self, level: LogLevel, message: String) {
        // Single extraction: get everything we need from self before any
        // filesystem I/O (critical for cross-principal path which does
        // create_dir_all + open).
        let capsule_id = self.capsule_id.as_str().to_owned();
        let invocation_principal = self
            .caller_context
            .as_ref()
            .and_then(|msg| msg.principal.as_deref())
            .and_then(|p| astrid_core::PrincipalId::new(p).ok())
            .filter(|p| *p != self.principal);
        let log_file = self.capsule_log.clone();

        let level_str = match level {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        };
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or_else(|_| "0".to_string(), |d| format!("{:.3}", d.as_secs_f64()));

        if let Some(ref inv_principal) = invocation_principal {
            // Cross-principal: open target log file, write, close.
            // No FD caching — append-mode open() is cheap, avoids leaks
            // in 1000-user deployments. OS filesystem cache handles the inode.
            if let Ok(home) = astrid_core::dirs::AstridHome::resolve() {
                let ph = home.principal_home(inv_principal);
                let log_dir = ph.log_dir().join(&capsule_id);
                let _ = std::fs::create_dir_all(&log_dir);
                let today = crate::engine::wasm::today_date_string();
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(log_dir.join(format!("{today}.log")))
                {
                    use std::io::Write;
                    let _ = writeln!(f, "{timestamp} {level_str} [{capsule_id}] {message}");
                }
            }
        } else if let Some(ref log_file) = log_file {
            // Default principal: use pre-opened log file (fast path).
            use std::io::Write;
            if let Ok(mut f) = log_file.lock() {
                let _ = writeln!(f, "{timestamp} {level_str} [{capsule_id}] {message}");
            }
        } else {
            match level {
                LogLevel::Trace => tracing::trace!(plugin = %capsule_id, "{message}"),
                LogLevel::Debug => tracing::debug!(plugin = %capsule_id, "{message}"),
                LogLevel::Info => tracing::info!(plugin = %capsule_id, "{message}"),
                LogLevel::Warn => tracing::warn!(plugin = %capsule_id, "{message}"),
                LogLevel::Error => tracing::error!(plugin = %capsule_id, "{message}"),
            }
        }
    }

    fn signal_ready(&mut self) {
        if let Some(tx) = &self.ready_tx {
            let _ = tx.send(true);
            tracing::debug!(
                capsule = %self.capsule_id,
                "Capsule signaled ready"
            );
        }
    }

    fn clock_ms(&mut self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0u64, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    fn check_capsule_capability(
        &mut self,
        request: CapabilityCheckRequest,
    ) -> Result<CapabilityCheckResponse, String> {
        let registry = self.capsule_registry.clone();
        let rt_handle = self.runtime_handle.clone();
        let host_semaphore = self.host_semaphore.clone();

        let allowed = if let Some(registry) = registry {
            if let Ok(source_uuid) = uuid::Uuid::parse_str(&request.source_uuid) {
                util::bounded_block_on(&rt_handle, &host_semaphore, async {
                    let reg = registry.read().await;
                    let Some(capsule_id) = reg.find_by_uuid(&source_uuid) else {
                        tracing::debug!(
                            uuid = %source_uuid,
                            capability = %request.capability,
                            "UUID not found in registry, denying capability"
                        );
                        return false;
                    };
                    let Some(capsule) = reg.get(capsule_id) else {
                        return false;
                    };
                    match request.capability.as_str() {
                        "allow_prompt_injection" => {
                            capsule.manifest().capabilities.allow_prompt_injection
                        },
                        other => {
                            tracing::warn!(
                                capability = %other,
                                "Unknown capability requested, denying"
                            );
                            false
                        },
                    }
                })
            } else {
                tracing::debug!(
                    uuid = %request.source_uuid,
                    "Malformed UUID in capability check, denying"
                );
                false
            }
        } else {
            false
        };

        Ok(CapabilityCheckResponse { allowed })
    }
}
