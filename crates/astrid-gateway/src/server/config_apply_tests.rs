use std::sync::Arc;

use astrid_config::{Config, IdentityLinkConfig, IdentitySection};
use astrid_core::identity::{FrontendType, IdentityStore, InMemoryIdentityStore};
use uuid::Uuid;

use crate::server::config_apply::{apply_identity_links, config_admin_id};

// --- config_admin_id ---

#[test]
fn config_admin_id_is_deterministic() {
    assert_eq!(config_admin_id(), config_admin_id());
    assert_ne!(config_admin_id(), Uuid::nil());
}

// --- apply_identity_links ---

fn make_cfg(links: Vec<IdentityLinkConfig>) -> Config {
    Config {
        identity: IdentitySection { links },
        ..Default::default()
    }
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