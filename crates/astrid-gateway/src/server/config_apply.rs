//! Config-driven startup helpers: identity pre-linking and connector validation.
//!
//! These functions are called during daemon startup to apply declarative
//! configuration — linking platform identities and validating connector
//! plugins — without requiring manual operator interaction after each restart.

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
// parse_frontend_type
// ---------------------------------------------------------------------------

/// Map a platform string to the corresponding [`FrontendType`].
///
/// Comparison is case-insensitive. Unknown strings fall through to
/// [`FrontendType::Custom`].
pub(super) fn parse_frontend_type(platform: &str) -> FrontendType {
    match platform.to_lowercase().as_str() {
        "discord" => FrontendType::Discord,
        "whatsapp" | "whats_app" => FrontendType::WhatsApp,
        "telegram" => FrontendType::Telegram,
        "slack" => FrontendType::Slack,
        "web" => FrontendType::Web,
        "cli" => FrontendType::Cli,
        other => FrontendType::Custom(other.to_owned()),
    }
}

// ---------------------------------------------------------------------------
// parse_connector_profile
// ---------------------------------------------------------------------------

/// Map a profile string to the corresponding [`ConnectorProfile`].
///
/// Comparison is case-insensitive. Unknown strings produce a `warn!` log and
/// default to [`ConnectorProfile::Chat`].
pub(super) fn parse_connector_profile(s: &str) -> ConnectorProfile {
    match s.to_lowercase().as_str() {
        "chat" => ConnectorProfile::Chat,
        "interactive" => ConnectorProfile::Interactive,
        "notify" => ConnectorProfile::Notify,
        "bridge" => ConnectorProfile::Bridge,
        other => {
            warn!(
                profile = other,
                "[[connectors]] unknown profile, defaulting to \"chat\""
            );
            ConnectorProfile::Chat
        },
    }
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
/// 3. If neither finds a match, mint a new identity via
///    [`IdentityStore::create_identity`].
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
             will create a new identity"
        );
    }

    // 2. Try display-name resolve via CLI frontend.
    if let Some(id) = identity_store
        .resolve(&FrontendType::Cli, astrid_user)
        .await
    {
        return Some(id);
    }

    // 3. Mint a fresh identity.
    // warn! (not info!) — creating an identity from config hints at a typo or
    // a first-run bootstrap that ops must be able to observe and confirm.
    match identity_store
        .create_identity(FrontendType::Cli, astrid_user)
        .await
    {
        Ok(id) => {
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

        let frontend = parse_frontend_type(&link_cfg.platform);

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

        // Build the link. `Uuid::nil()` for admin_id signals "pre-configured
        // by daemon config" rather than a real admin user performing the link.
        let link = FrontendLink::new(
            astrid_id.id,
            frontend,
            &link_cfg.platform_user_id,
            LinkVerificationMethod::AdminLink {
                admin_id: Uuid::nil(),
            },
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
/// 2. Plugin is present in the registry (i.e. loaded successfully).
/// 3. The plugin exposes a connector with the declared profile.
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
        if registry.get(&pid).is_none() {
            warnings.push(format!(
                "[[connectors]] plugin not loaded: {}",
                conn_cfg.plugin
            ));
            continue;
        }
        let expected_profile = parse_connector_profile(&conn_cfg.profile);
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

    // --- parse_frontend_type ---

    #[test]
    fn test_parse_frontend_type_known() {
        assert_eq!(parse_frontend_type("discord"), FrontendType::Discord);
        assert_eq!(parse_frontend_type("whatsapp"), FrontendType::WhatsApp);
        assert_eq!(parse_frontend_type("whats_app"), FrontendType::WhatsApp);
        assert_eq!(parse_frontend_type("telegram"), FrontendType::Telegram);
        assert_eq!(parse_frontend_type("slack"), FrontendType::Slack);
        assert_eq!(parse_frontend_type("web"), FrontendType::Web);
        assert_eq!(parse_frontend_type("cli"), FrontendType::Cli);
        // Case-insensitive.
        assert_eq!(parse_frontend_type("TELEGRAM"), FrontendType::Telegram);
        assert_eq!(parse_frontend_type("Discord"), FrontendType::Discord);
    }

    #[test]
    fn test_parse_frontend_type_custom() {
        assert_eq!(
            parse_frontend_type("matrix"),
            FrontendType::Custom("matrix".to_owned())
        );
        // Input is lowercased before storing in Custom.
        assert_eq!(
            parse_frontend_type("MyPlatform"),
            FrontendType::Custom("myplatform".to_owned())
        );
    }

    // --- parse_connector_profile ---

    #[test]
    fn test_parse_connector_profile_known() {
        assert_eq!(parse_connector_profile("chat"), ConnectorProfile::Chat);
        assert_eq!(
            parse_connector_profile("interactive"),
            ConnectorProfile::Interactive
        );
        assert_eq!(parse_connector_profile("notify"), ConnectorProfile::Notify);
        assert_eq!(parse_connector_profile("bridge"), ConnectorProfile::Bridge);
    }

    #[test]
    fn test_parse_connector_profile_unknown_warns() {
        // Unknown string → defaults to Chat (warn! is a side-effect).
        assert_eq!(
            parse_connector_profile("unknown-profile"),
            ConnectorProfile::Chat
        );
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
