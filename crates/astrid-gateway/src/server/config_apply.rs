//! Config-driven startup helpers: identity pre-linking and connector validation.
//!
//! These functions are called during daemon startup to apply declarative
//! configuration — linking platform identities and validating connector
//! plugins — without requiring manual operator interaction after each restart.

use std::str::FromStr;
use std::sync::Arc;

use astrid_capsule::registry::CapsuleRegistry;
use astrid_config::{Config, ConnectorConfig};
use astrid_core::ConnectorProfile;
use astrid_core::ConnectorSource;
use astrid_core::identity::{
    AstridUserId, FrontendLink, FrontendType, IdentityError, IdentityStore, LinkVerificationMethod,
};
use tracing::{debug, info, warn};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the current OS username.
fn get_os_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "root".to_string())
}

// ---------------------------------------------------------------------------
// config_admin_id
// ---------------------------------------------------------------------------

/// Deterministic admin UUID for config-originated identity links.
///
/// Computed via UUID v5 (SHA-1 namespace) so it is stable across restarts
/// without requiring an identity store entry. Used only as audit metadata
/// in [`LinkVerificationMethod::AdminLink`].
fn config_admin_id() -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, b"astrid:config-admin")
}

// ---------------------------------------------------------------------------
// resolve_astrid_user
// ---------------------------------------------------------------------------

/// Resolve or create the canonical [`AstridUserId`] for the given string.
///
/// Resolution order:
/// 1. If `astrid_user` is a valid UUID, look up by ID via
///    [`IdentityStore::get_by_id`].
/// 2. Otherwise, try [`IdentityStore::resolve`] against the `cli` frontend
///    (display-name lookup).
/// 3. If "root" is specified, try resolving via the current OS username.
/// 4. If neither finds a match, mint a new identity via
///    [`IdentityStore::create_identity`] and set the display name.
///
/// **Note on restart behaviour:** When a display name is supplied and the
/// in-memory store is wiped on exit, a new `AstridUserId` is minted on each
/// restart. This is acceptable for the current in-memory implementation; the
/// planned `SurrealDB`-backed store will make `resolve()` find the existing
/// identity on restart, making the UUID stable across restarts.
async fn resolve_astrid_user(
    identity_store: &Arc<dyn IdentityStore>,
    astrid_user: &str,
) -> Option<AstridUserId> {
    // 1. Try UUID parse — look up the existing identity by its canonical ID.
    if let Ok(uuid) = Uuid::parse_str(astrid_user) {
        if let Some(id) = identity_store.get_by_id(uuid).await {
            return Some(id);
        }
        warn!(
            astrid_user = astrid_user,
            "[[identity.links]] UUID not found in identity store; \
             will create a new identity using this UUID string as the display name. \
             Note: The actual system identity ID will be a fresh random UUID."
        );
    }

    // 2. Try display-name resolve via CLI frontend.
    if let Some(id) = identity_store
        .resolve(&FrontendType::Cli, astrid_user)
        .await
    {
        return Some(id);
    }

    // 3. Root user resolution (OS username fallback).
    if astrid_user == "root" {
        let os_user = get_os_username();
        if let Some(id) = identity_store.resolve(&FrontendType::Cli, &os_user).await {
            return Some(id);
        }
    }

    // 4. Mint a fresh identity via CLI frontend (avoids double-linking — the
    //    platform link is created separately in apply_identity_links).
    let final_username = if astrid_user == "root" {
        get_os_username()
    } else {
        astrid_user.to_string()
    };

    match identity_store
        .create_identity(FrontendType::Cli, &final_username)
        .await
    {
        Ok(id) => {
            // Set display name from the config's astrid_user field.
            let updated = AstridUserId {
                display_name: Some(final_username),
                ..id.clone()
            };
            if let Err(e) = identity_store.update_identity(updated).await {
                warn!(
                    astrid_user = astrid_user,
                    error = %e,
                    "[[identity.links]] created identity but failed to set display name"
                );
            }

            warn!(
                astrid_user = astrid_user,
                "[[identity.links]] astrid_user not found in store; \
                 created new identity (verify this is not a typo or misconfiguration)"
            );
            Some(id)
        },
        Err(e) => {
            warn!(
                astrid_user = astrid_user,
                error = %e,
                "[[identity.links]] failed to create identity; skipping link"
            );
            None
        },
    }
}

// ---------------------------------------------------------------------------
// apply_identity_links
// ---------------------------------------------------------------------------

/// Apply all `[[identity.links]]` entries from the configuration.
///
/// Called during daemon startup after the identity store is created. Each call
/// is idempotent — if a link already exists it is silently skipped.
pub(super) async fn apply_identity_links(cfg: &Config, identity_store: &Arc<dyn IdentityStore>) {
    let admin_id = config_admin_id();

    for link_cfg in &cfg.identity.links {
        // Validate required fields.
        if link_cfg.platform.is_empty()
            || link_cfg.platform_user_id.is_empty()
            || link_cfg.astrid_user.is_empty()
        {
            warn!(
                platform = %link_cfg.platform,
                platform_user_id = %link_cfg.platform_user_id,
                astrid_user = %link_cfg.astrid_user,
                "[[identity.links]] entry has empty required field; skipping"
            );
            continue;
        }

        // Validate link method — only "admin" is currently supported.
        if link_cfg.method != "admin" {
            warn!(
                platform = %link_cfg.platform,
                method = %link_cfg.method,
                "[[identity.links]] unsupported link method (only \"admin\" is supported); skipping"
            );
            continue;
        }

        // FrontendType::from_str is infallible — unknown strings become Custom.
        let frontend = FrontendType::from_str(&link_cfg.platform).unwrap_or_else(|e| match e {});

        // Idempotency check: skip if the platform_user_id is already linked.
        if identity_store
            .resolve(&frontend, &link_cfg.platform_user_id)
            .await
            .is_some()
        {
            debug!(
                platform = %link_cfg.platform,
                platform_user_id = %link_cfg.platform_user_id,
                "[[identity.links]] link already exists; skipping"
            );
            continue;
        }

        // Resolve or create the target Astrid identity.
        let Some(astrid_id) = resolve_astrid_user(identity_store, &link_cfg.astrid_user).await
        else {
            continue;
        };

        // Build the link. config_admin_id() is a deterministic UUID v5 that
        // signals "pre-configured by daemon config" in audit trails.
        let link = FrontendLink::new(
            astrid_id.id,
            frontend,
            &link_cfg.platform_user_id,
            LinkVerificationMethod::AdminLink { admin_id },
            false, // not primary — CLI link is primary
        );

        match identity_store.create_link(link).await {
            Ok(()) => {
                info!(
                    platform = %link_cfg.platform,
                    platform_user_id = %link_cfg.platform_user_id,
                    astrid_user = %link_cfg.astrid_user,
                    "Pre-configured identity link applied"
                );
            },
            Err(IdentityError::FrontendAlreadyLinked { .. }) => {
                // Race between the idempotency check above and create_link; harmless.
                debug!(
                    platform = %link_cfg.platform,
                    platform_user_id = %link_cfg.platform_user_id,
                    "[[identity.links]] link already exists (race); skipping"
                );
            },
            Err(e) => {
                warn!(
                    platform = %link_cfg.platform,
                    platform_user_id = %link_cfg.platform_user_id,
                    astrid_user = %link_cfg.astrid_user,
                    error = %e,
                    "[[identity.links]] failed to create link"
                );
            },
        }
    }
}

// ---------------------------------------------------------------------------
// validate_connector_declarations
// ---------------------------------------------------------------------------

/// Validate `[[connectors]]` declarations against the loaded plugin registry.
///
/// Called synchronously after acquiring a read lock on the registry. Returns
/// a `Vec<String>` of human-readable warning messages so the caller can log
/// them — keeping this function pure and unit-testable without a live registry.
///
/// Checks (in order):
/// 1. Plugin ID format is valid.
/// 2. Connector profile string is valid.
/// 3. Plugin is present in the registry (i.e. loaded successfully).
/// 4. The plugin exposes a connector with the declared profile.
pub(super) fn validate_connector_declarations(
    connectors: &[ConnectorConfig],
    registry: &CapsuleRegistry,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for conn_cfg in connectors {
        if conn_cfg.plugin.is_empty() {
            continue;
        }
        let Ok(pid) = astrid_capsule::capsule::CapsuleId::new(conn_cfg.plugin.clone()) else {
            warnings.push(format!(
                "[[connectors]] invalid plugin ID format: {}",
                conn_cfg.plugin
            ));
            continue;
        };
        let expected_profile = match ConnectorProfile::from_str(&conn_cfg.profile) {
            Ok(p) => p,
            Err(e) => {
                warnings.push(format!(
                    "[[connectors]] plugin '{}': {}",
                    conn_cfg.plugin, e
                ));
                continue;
            },
        };

        // Special case: "native-cli" is a synthetic connector registered by the
        // daemon directly, not via a plugin in the registry.
        if conn_cfg.plugin == "native-cli" {
            // "native-cli" is Interactive; if user declares it as something else
            // (like Bridge), warn them about the mismatch.
            if expected_profile != ConnectorProfile::Interactive {
                warnings.push(format!(
                    "[[connectors]] plugin 'native-cli' has fixed profile 'interactive' (config declared '{}')",
                    conn_cfg.profile
                ));
            }
            continue;
        }

        if registry.get(&pid).is_none() {
            warnings.push(format!(
                "[[connectors]] plugin not loaded: {}",
                conn_cfg.plugin
            ));
            continue;
        }
        let has_match = registry.all_connector_descriptors().iter().any(|d| {
            let from_plugin = match &d.source {
                ConnectorSource::Wasm { capsule_id } | ConnectorSource::OpenClaw { capsule_id } => {
                    capsule_id.as_str() == conn_cfg.plugin.as_str()
                },
                ConnectorSource::Native => false,
            };
            from_plugin && d.profile == expected_profile
        });
        if !has_match {
            warnings.push(format!(
                "[[connectors]] plugin '{}' loaded but no connector with profile '{}' found",
                conn_cfg.plugin, conn_cfg.profile
            ));
        }
    }
    warnings
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
