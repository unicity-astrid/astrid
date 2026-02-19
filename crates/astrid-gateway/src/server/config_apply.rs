//! Config-driven startup helpers: identity pre-linking and connector validation.
//!
//! These functions are called during daemon startup to apply declarative
//! configuration — linking platform identities and validating connector
//! plugins — without requiring manual operator interaction after each restart.

use std::sync::Arc;

use astrid_config::Config;
use astrid_core::ConnectorProfile;
use astrid_core::error::SecurityError;
use astrid_core::identity::{
    AstridUserId, FrontendLink, FrontendType, IdentityStore, LinkVerificationMethod,
};
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
    match identity_store
        .create_identity(FrontendType::Cli, astrid_user)
        .await
    {
        Ok(id) => Some(id),
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
}
