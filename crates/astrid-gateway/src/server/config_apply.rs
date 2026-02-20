//! Config-driven startup helpers: identity pre-linking and connector validation.
//!
//! These functions are called during daemon startup to apply declarative
//! configuration — linking platform identities and validating connector
//! plugins — without requiring manual operator interaction after each restart.

use std::str::FromStr;
use std::sync::Arc;

use astrid_config::{Config, ConnectorConfig};
use astrid_core::ConnectorProfile;
use astrid_core::ConnectorSource;
use astrid_core::error::SecurityError;
use astrid_core::identity::{
    AstridUserId, FrontendLink, FrontendType, IdentityStore, LinkVerificationMethod,
};
use astrid_plugins::PluginRegistry;
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
            Err(SecurityError::FrontendAlreadyLinked { .. }) => {
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
    registry: &PluginRegistry,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for conn_cfg in connectors {
        if conn_cfg.plugin.is_empty() {
            continue;
        }
        let Ok(pid) = astrid_plugins::PluginId::new(conn_cfg.plugin.clone()) else {
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
        if registry.get(&pid).is_none() {
            warnings.push(format!(
                "[[connectors]] plugin not loaded: {}",
                conn_cfg.plugin
            ));
            continue;
        }
        let has_match = registry.all_connector_descriptors().iter().any(|d| {
            let from_plugin = match &d.source {
                ConnectorSource::Wasm { plugin_id } | ConnectorSource::OpenClaw { plugin_id } => {
                    plugin_id.as_str() == conn_cfg.plugin.as_str()
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

#[cfg(test)]
mod tests {
    use astrid_config::{Config, IdentityLinkConfig, IdentitySection};
    use astrid_core::identity::InMemoryIdentityStore;

    use super::*;

    // --- config_admin_id ---

    #[test]
    fn config_admin_id_is_deterministic() {
        assert_eq!(config_admin_id(), config_admin_id());
        assert_ne!(config_admin_id(), Uuid::nil());
    }

    // --- apply_identity_links ---

    fn make_cfg(links: Vec<IdentityLinkConfig>) -> Config {
        let mut cfg = Config::default();
        cfg.identity = IdentitySection { links };
        cfg
    }

    fn make_link(platform: &str, platform_user_id: &str, astrid_user: &str) -> IdentityLinkConfig {
        IdentityLinkConfig {
            platform: platform.to_owned(),
            platform_user_id: platform_user_id.to_owned(),
            astrid_user: astrid_user.to_owned(),
            method: "admin".to_owned(),
        }
    }

    #[tokio::test]
    async fn test_apply_identity_links_creates_link() {
        let store: Arc<dyn IdentityStore> = Arc::new(InMemoryIdentityStore::new());
        let cfg = make_cfg(vec![make_link("telegram", "123456", "josh")]);

        apply_identity_links(&cfg, &store).await;

        let resolved = store.resolve(&FrontendType::Telegram, "123456").await;
        assert!(resolved.is_some(), "link should have been created");
    }

    #[tokio::test]
    async fn test_apply_identity_links_sets_display_name() {
        let store: Arc<dyn IdentityStore> = Arc::new(InMemoryIdentityStore::new());
        let cfg = make_cfg(vec![make_link("telegram", "123456", "josh")]);

        apply_identity_links(&cfg, &store).await;

        // Resolve the linked identity and verify display name was set.
        let resolved = store.resolve(&FrontendType::Telegram, "123456").await;
        assert!(resolved.is_some());
        let user = resolved.unwrap();
        assert_eq!(user.display_name.as_deref(), Some("josh"));
    }

    #[tokio::test]
    async fn test_apply_identity_links_idempotent() {
        let store: Arc<dyn IdentityStore> = Arc::new(InMemoryIdentityStore::new());
        let cfg = make_cfg(vec![make_link("telegram", "123456", "josh")]);

        apply_identity_links(&cfg, &store).await;
        apply_identity_links(&cfg, &store).await; // must not error or duplicate

        let resolved = store.resolve(&FrontendType::Telegram, "123456").await;
        assert!(
            resolved.is_some(),
            "link should still exist after second call"
        );
    }

    #[tokio::test]
    async fn test_apply_identity_links_skips_incomplete() {
        let store: Arc<dyn IdentityStore> = Arc::new(InMemoryIdentityStore::new());
        let cfg = make_cfg(vec![
            make_link("", "123", "josh"),      // missing platform
            make_link("telegram", "", "josh"), // missing platform_user_id
            make_link("telegram", "456", ""),  // missing astrid_user
        ]);

        apply_identity_links(&cfg, &store).await; // must not panic

        assert!(
            store
                .resolve(&FrontendType::Telegram, "123")
                .await
                .is_none()
        );
        assert!(
            store
                .resolve(&FrontendType::Telegram, "456")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn apply_identity_links_uuid_resolution() {
        let store: Arc<dyn IdentityStore> = Arc::new(InMemoryIdentityStore::new());
        // Pre-create an identity so we have a valid UUID to reference.
        let existing = store
            .create_identity(FrontendType::Cli, "known-user")
            .await
            .expect("create_identity");
        let uuid_str = existing.id.to_string();

        // Use the UUID string as astrid_user — should resolve via get_by_id.
        let cfg = make_cfg(vec![make_link("telegram", "998", &uuid_str)]);
        apply_identity_links(&cfg, &store).await;

        let resolved = store.resolve(&FrontendType::Telegram, "998").await;
        assert!(
            resolved.is_some(),
            "link should have been created via UUID lookup"
        );
        assert_eq!(
            resolved.unwrap().id,
            existing.id,
            "should resolve to the pre-existing identity"
        );
    }

    #[tokio::test]
    async fn apply_identity_links_creates_new_user_with_warn() {
        // "brand-new-user" is not in the store; resolve_astrid_user mints a fresh
        // identity (step 3) and emits a warn! about possible misconfiguration.
        let store: Arc<dyn IdentityStore> = Arc::new(InMemoryIdentityStore::new());
        let cfg = make_cfg(vec![make_link("discord", "555", "brand-new-user")]);
        apply_identity_links(&cfg, &store).await;

        // The new identity should be linked to discord/555.
        let resolved = store.resolve(&FrontendType::Discord, "555").await;
        assert!(
            resolved.is_some(),
            "new identity should have been created and linked"
        );
    }

    #[tokio::test]
    async fn apply_identity_links_skips_unsupported_method() {
        let store: Arc<dyn IdentityStore> = Arc::new(InMemoryIdentityStore::new());
        let cfg = make_cfg(vec![IdentityLinkConfig {
            platform: "telegram".to_owned(),
            platform_user_id: "777".to_owned(),
            astrid_user: "carol".to_owned(),
            method: "oauth".to_owned(), // unsupported
        }]);

        apply_identity_links(&cfg, &store).await;

        assert!(
            store
                .resolve(&FrontendType::Telegram, "777")
                .await
                .is_none(),
            "link should NOT be created for unsupported method"
        );
    }

    #[tokio::test]
    async fn apply_identity_links_two_platforms_same_user() {
        let store: Arc<dyn IdentityStore> = Arc::new(InMemoryIdentityStore::new());
        let cfg = make_cfg(vec![
            make_link("telegram", "111", "shared-user"),
            make_link("discord", "222", "shared-user"),
        ]);

        apply_identity_links(&cfg, &store).await;

        let telegram_id = store.resolve(&FrontendType::Telegram, "111").await;
        let discord_id = store.resolve(&FrontendType::Discord, "222").await;
        assert!(telegram_id.is_some(), "telegram link should exist");
        assert!(discord_id.is_some(), "discord link should exist");
        // Both should map to the same canonical Astrid identity.
        assert_eq!(
            telegram_id.unwrap().id,
            discord_id.unwrap().id,
            "both platforms must resolve to the same AstridUserId"
        );
    }

    #[tokio::test]
    async fn apply_identity_links_root_user_resolution() {
        let store: Arc<dyn IdentityStore> = Arc::new(InMemoryIdentityStore::new());
        let os_user = get_os_username();

        // config uses "root"
        let cfg = make_cfg(vec![make_link("telegram", "888", "root")]);
        apply_identity_links(&cfg, &store).await;

        let resolved = store.resolve(&FrontendType::Telegram, "888").await;
        assert!(resolved.is_some());
        let user = resolved.unwrap();

        // Identity should be created with OS username as CLI platform ID.
        let cli_id = store.resolve(&FrontendType::Cli, &os_user).await;
        assert!(cli_id.is_some());
        assert_eq!(user.id, cli_id.unwrap().id);
        assert_eq!(user.display_name.as_deref(), Some(os_user.as_str()));
    }

    // --- validate_connector_declarations ---

    // Minimal mock plugin for validate_connector_declarations tests.
    struct MockPlugin {
        id: astrid_plugins::PluginId,
        manifest: astrid_plugins::PluginManifest,
    }

    impl MockPlugin {
        fn new(id: &str) -> Self {
            let plugin_id = astrid_plugins::PluginId::from_static(id);
            Self {
                manifest: astrid_plugins::PluginManifest {
                    id: plugin_id.clone(),
                    name: format!("Mock {id}"),
                    version: "0.1.0".into(),
                    description: None,
                    author: None,
                    entry_point: astrid_plugins::PluginEntryPoint::Wasm {
                        path: "plugin.wasm".into(),
                        hash: None,
                    },
                    capabilities: vec![],
                    connectors: vec![],
                    config: std::collections::HashMap::new(),
                },
                id: plugin_id,
            }
        }
    }

    #[async_trait::async_trait]
    impl astrid_plugins::Plugin for MockPlugin {
        fn id(&self) -> &astrid_plugins::PluginId {
            &self.id
        }

        fn manifest(&self) -> &astrid_plugins::PluginManifest {
            &self.manifest
        }

        fn state(&self) -> astrid_plugins::PluginState {
            astrid_plugins::PluginState::Ready
        }

        async fn load(
            &mut self,
            _ctx: &astrid_plugins::PluginContext,
        ) -> astrid_plugins::PluginResult<()> {
            Ok(())
        }

        async fn unload(&mut self) -> astrid_plugins::PluginResult<()> {
            Ok(())
        }

        fn tools(&self) -> &[Arc<dyn astrid_plugins::PluginTool>] {
            &[]
        }

        fn connectors(&self) -> &[astrid_core::ConnectorDescriptor] {
            &[]
        }
    }

    #[test]
    fn validate_connector_declarations_warns_missing() {
        let registry = PluginRegistry::new();
        let connectors = vec![ConnectorConfig {
            plugin: "missing-plugin".to_owned(),
            profile: "chat".to_owned(),
        }];
        let warnings = validate_connector_declarations(&connectors, &registry);
        assert!(
            !warnings.is_empty(),
            "should warn when plugin is not loaded"
        );
        assert!(
            warnings.iter().any(|w| w.contains("missing-plugin")),
            "warning should name the missing plugin"
        );
    }

    #[test]
    fn validate_connector_declarations_warns_invalid_profile() {
        let registry = PluginRegistry::new();
        let connectors = vec![ConnectorConfig {
            plugin: "some-plugin".to_owned(),
            profile: "bogus".to_owned(),
        }];
        let warnings = validate_connector_declarations(&connectors, &registry);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("unknown connector profile")),
            "should warn about invalid profile: {warnings:?}"
        );
    }

    #[test]
    fn validate_connector_declarations_ok_known() {
        let mut registry = PluginRegistry::new();
        let plugin_id = astrid_plugins::PluginId::from_static("known-connector-plugin");
        registry
            .register(Box::new(MockPlugin::new("known-connector-plugin")))
            .unwrap();
        let descriptor =
            astrid_core::ConnectorDescriptor::builder("My Connector", FrontendType::Telegram)
                .source(ConnectorSource::new_wasm("known-connector-plugin").unwrap())
                .profile(ConnectorProfile::Chat)
                .build();
        registry.register_connector(&plugin_id, descriptor).unwrap();

        let connectors = vec![ConnectorConfig {
            plugin: "known-connector-plugin".to_owned(),
            profile: "chat".to_owned(),
        }];
        let warnings = validate_connector_declarations(&connectors, &registry);
        assert!(
            warnings.is_empty(),
            "no warnings expected for a properly registered connector: {warnings:?}"
        );
    }
}
