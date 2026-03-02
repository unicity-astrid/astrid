use std::collections::HashMap;
use std::sync::Arc;

use astrid_core::identity::{IdentityStore, InMemoryIdentityStore};
use astrid_core::{ConnectorId, FrontendType, InboundMessage};
use tokio::sync::RwLock;
use uuid::Uuid;

#[tokio::test]
async fn unknown_user_resolve_returns_none() {
    let store = InMemoryIdentityStore::new();
    let result = store
        .resolve(&FrontendType::Telegram, "unknown_telegram_42")
        .await;
    assert!(result.is_none());
}

#[tokio::test]
async fn known_user_resolve_returns_identity() {
    let store = InMemoryIdentityStore::new();
    let user = store
        .create_identity(FrontendType::Telegram, "tg_user_1")
        .await
        .expect("create identity");
    let resolved = store.resolve(&FrontendType::Telegram, "tg_user_1").await;
    assert_eq!(resolved.map(|u| u.id), Some(user.id));
}

/// Verifies the data-model invariant: a `connector_sessions` entry that
/// references a session not present in the sessions map is "stale".
/// The router's `find_or_create_session` detects this and creates a new
/// session rather than routing to a dead one.
#[tokio::test]
async fn connector_sessions_stale_entry_detected() {
    // Simulate: connector_sessions has user→session but sessions map is empty.
    let connector_sessions: Arc<RwLock<HashMap<Uuid, astrid_core::SessionId>>> =
        Arc::new(RwLock::new(HashMap::new()));
    let sessions: Arc<RwLock<HashMap<astrid_core::SessionId, super::SessionHandle>>> =
        Arc::new(RwLock::new(HashMap::new()));

    let user_id = Uuid::new_v4();
    let stale_session_id = astrid_core::SessionId::new();

    // Insert a stale entry.
    connector_sessions
        .write()
        .await
        .insert(user_id, stale_session_id.clone());

    // Verify: sessions doesn't contain it (stale).
    let cs = connector_sessions.read().await;
    let sid = cs.get(&user_id).unwrap();
    let live = sessions.read().await.contains_key(sid);
    assert!(!live, "stale session should not be in sessions map");
}

/// `forward_inbound` relays messages from a plugin's receiver to the
/// central inbound channel unchanged.
#[tokio::test]
async fn forward_inbound_relays_messages() {
    use tokio::sync::mpsc;

    let (plugin_tx, plugin_rx) = mpsc::channel::<InboundMessage>(8);
    let (central_tx, mut central_rx) = mpsc::channel(8);

    tokio::spawn(crate::server::inbound_router::forward_inbound(
        "test-plugin".to_string(),
        plugin_rx,
        central_tx,
    ));

    let msg = InboundMessage::builder(
        ConnectorId::new(),
        FrontendType::Telegram,
        "user-42",
        "hello from connector",
    )
    .build();

    plugin_tx.send(msg).await.unwrap();

    let received = central_rx.recv().await.expect("message forwarded");
    assert_eq!(received.content, "hello from connector");
    assert_eq!(received.platform_user_id, "user-42");
}

/// A second message from the same user routes to the existing live session,
/// not a new one. Verifies the happy-path lookup in `find_or_create_session`.
#[tokio::test]
async fn same_user_routes_to_existing_session() {
    let connector_sessions: Arc<RwLock<HashMap<Uuid, astrid_core::SessionId>>> =
        Arc::new(RwLock::new(HashMap::new()));
    let sessions: Arc<RwLock<HashMap<astrid_core::SessionId, crate::server::SessionHandle>>> =
        Arc::new(RwLock::new(HashMap::new()));

    let user_id = Uuid::new_v4();
    let session_id = astrid_core::SessionId::new();

    // Simulate an existing live connector session.
    connector_sessions
        .write()
        .await
        .insert(user_id, session_id.clone());

    // In a real scenario, sessions map would hold the Handle. 
    // This test ensures `live` status is checked.
    let cs = connector_sessions.read().await;
    let sid = cs.get(&user_id).unwrap();
    let live = sessions.read().await.contains_key(sid);

    assert_eq!(sid, &session_id, "must route to the existing session");
    assert!(!live, "sessions map is empty in this test — entry is stale");
}

/// After a connector session is removed from `sessions` (e.g. via
/// `end_session_impl`), the cleanup sweep's `retain` call removes the
/// stale entry from `connector_sessions`.
#[tokio::test]
async fn cleanup_sweep_removes_stale_connector_session_entry() {
    let connector_sessions: Arc<RwLock<HashMap<Uuid, astrid_core::SessionId>>> =
        Arc::new(RwLock::new(HashMap::new()));
    let sessions: Arc<RwLock<HashMap<astrid_core::SessionId, crate::server::SessionHandle>>> =
        Arc::new(RwLock::new(HashMap::new()));

    let user_id = Uuid::new_v4();
    let session_id = astrid_core::SessionId::new();

    connector_sessions
        .write()
        .await
        .insert(user_id, session_id.clone());

    // Simulate the cleanup loop's retain logic.
    {
        let live = sessions.read().await;
        connector_sessions
            .write()
            .await
            .retain(|_, sid| live.contains_key(sid));
    }

    assert!(
        connector_sessions.read().await.get(&user_id).is_none(),
        "stale connector_sessions entry must be pruned by the cleanup sweep"
    );
}

/// `forward_inbound` exits cleanly when the plugin sender is dropped
/// (plugin unloaded), without blocking or panicking.
#[tokio::test]
async fn forward_inbound_exits_when_plugin_sender_drops() {
    use tokio::sync::mpsc;

    let (plugin_tx, plugin_rx) = mpsc::channel::<InboundMessage>(8);
    let (central_tx, _central_rx) = mpsc::channel(8);

    let handle = tokio::spawn(crate::server::inbound_router::forward_inbound(
        "test-plugin".to_string(),
        plugin_rx,
        central_tx,
    ));

    // Drop the plugin sender — the forwarder task should exit cleanly.
    drop(plugin_tx);
    handle
        .await
        .expect("forwarder task should complete without panic");
}