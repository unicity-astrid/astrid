#[cfg(test)]
mod tests {
    use crate::server::{DaemonServer, DaemonStartOptions};
    use astrid_core::InboundMessage;
    use astrid_core::connector::ConnectorId;
    use astrid_core::dirs::AstridHome;
    use astrid_core::identity::FrontendType;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_config_driven_identity_link_boot() {
        let temp_home = TempDir::new().unwrap();
        let temp_ws = TempDir::new().unwrap();

        let home = AstridHome::from_path(temp_home.path());

        // Write a config file with an identity link
        let config_path = home.config_path();
        let config_content = r#"
[model]
provider = "openai-compat"
api_url = "http://localhost:1234/v1"
api_key = "test-key"

[[identity.links]]
platform = "telegram"
platform_user_id = "tg-123"
astrid_user = "josh"
method = "admin"
"#;
        std::fs::write(&config_path, config_content).unwrap();

        // Start the daemon
        let (daemon, _handle, _addr, _cfg) = DaemonServer::start(
            DaemonStartOptions {
                ephemeral: true,
                workspace_root: Some(temp_ws.path().to_path_buf()),
                ..Default::default()
            },
            Some(home),
        )
        .await
        .expect("Failed to start daemon");

        // Verify that the identity link was applied by sending an inbound message
        let msg = InboundMessage::builder(
            ConnectorId::new(),
            FrontendType::Telegram,
            "tg-123",
            "Hello Astrid!",
        )
        .build();

        daemon.inbound_tx.send(msg).await.unwrap();

        // Give the inbound router a moment to process
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Check if a session was created for the user
        let sessions = daemon.sessions.read().await;
        assert!(
            !sessions.is_empty(),
            "A session should have been created for the pre-linked user"
        );

        // Verify the session is linked to the correct user
        let session_handle = sessions.values().next().unwrap();
        assert!(
            session_handle.user_id.is_some(),
            "Session should have a user ID"
        );

        // Resolve josh's identity to compare UUIDs
        let josh = daemon
            .identity_store
            .resolve(&FrontendType::Cli, "josh")
            .await
            .expect("josh should exist");
        assert_eq!(session_handle.user_id, Some(josh.id));
    }

    #[tokio::test]
    async fn test_config_driven_connector_validation() {
        let temp_home = TempDir::new().unwrap();
        let temp_ws = TempDir::new().unwrap();

        let home = AstridHome::from_path(temp_home.path());
        home.ensure().unwrap();

        // Create a mock plugin directory
        let plugins_dir = home.plugins_dir();
        std::fs::create_dir_all(&plugins_dir).unwrap();

        // Write a mock plugin manifest
        let plugin_path = plugins_dir.join("test-plugin");
        std::fs::create_dir_all(&plugin_path).unwrap();
        let manifest_content = r#"
id = "test-plugin"
name = "Test Plugin"
version = "0.1.0"

[entry_point]
type = "wasm"
path = "plugin.wasm"

[[connectors]]
name = "Test Connector"
platform = "telegram"
profile = "chat"
"#;
        std::fs::write(plugin_path.join("plugin.toml"), manifest_content).unwrap();
        // Create a dummy wasm file so it doesn't fail discovery
        std::fs::write(plugin_path.join("plugin.wasm"), b"").unwrap();

        // Write a config file with a connector declaration
        let config_path = home.config_path();
        let config_content = r#"
[model]
provider = "openai-compat"
api_url = "http://localhost:1234/v1"
api_key = "test-key"

[[connectors]]
plugin = "test-plugin"
profile = "chat"

[[connectors]]
plugin = "missing-plugin"
profile = "chat"
"#;
        std::fs::write(&config_path, config_content).unwrap();

        // Start the daemon
        let (_daemon, _handle, _addr, cfg) = DaemonServer::start(
            DaemonStartOptions {
                ephemeral: true,
                workspace_root: Some(temp_ws.path().to_path_buf()),
                ..Default::default()
            },
            Some(home),
        )
        .await
        .expect("Failed to start daemon");

        // Verify config was loaded
        assert_eq!(cfg.connectors.len(), 2);
        assert_eq!(cfg.connectors[0].plugin, "test-plugin");
        assert_eq!(cfg.connectors[1].plugin, "missing-plugin");

        // Note: validation happens in a background task after auto-load.
        // We can't easily capture the log warnings here, but we've verified
        // the config is loaded and passed through.
        // The pure validation logic is already unit-tested in config_apply.rs.
    }

    #[tokio::test]
    async fn test_connector_session_rpc_isolation() {
        use crate::rpc::AstridRpcClient;
        use jsonrpsee::core::client::Error as JsonRpcError;
        use jsonrpsee::ws_client::WsClientBuilder;

        let temp_home = TempDir::new().unwrap();
        let temp_ws = TempDir::new().unwrap();

        let home = AstridHome::from_path(temp_home.path());
        let config_path = home.config_path();
        let config_content = r#"
[model]
provider = "openai-compat"
api_url = "http://localhost:1234/v1"
api_key = "test-key"

[[identity.links]]
platform = "telegram"
platform_user_id = "tg-isolation-test"
astrid_user = "isolation-tester"
method = "admin"
"#;
        std::fs::write(&config_path, config_content).unwrap();

        let (daemon, _handle, addr, _cfg) = DaemonServer::start(
            DaemonStartOptions {
                ephemeral: true,
                workspace_root: Some(temp_ws.path().to_path_buf()),
                ..Default::default()
            },
            Some(home.clone()),
        )
        .await
        .expect("Failed to start daemon");

        // 1. Send an inbound message to create a connector session.
        let msg = InboundMessage::builder(
            ConnectorId::new(),
            FrontendType::Telegram,
            "tg-isolation-test",
            "Hello isolation!",
        )
        .build();

        daemon.inbound_tx.send(msg).await.unwrap();

        // Give it a moment to process and create the session by polling.
        let mut session_id = None;
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let sessions = daemon.sessions.read().await;
            if sessions.len() == 1 {
                let (id, handle) = sessions.iter().next().unwrap();
                assert!(
                    handle.user_id.is_some(),
                    "Expected session to have a user ID (connector session)"
                );
                session_id = Some(id.clone());
                break;
            }
        }

        let session_id = session_id
            .expect("Expected exactly one connector session to be created within the timeout");

        // 2. Connect a mock CLI RPC client.
        let url = format!("ws://{addr}");
        let client = WsClientBuilder::default()
            .build(&url)
            .await
            .expect("Failed to connect via RPC");

        // 3. Attempt to cancel the connector session's turn via RPC.
        // This should fail with INVALID_REQUEST because the session belongs to a connector.
        let cancel_res = client.cancel_turn(session_id.clone()).await;
        let err = cancel_res.expect_err("cancel_turn on a connector session should fail");
        match err {
            JsonRpcError::Call(e) => {
                assert_eq!(e.code(), crate::rpc::error_codes::INVALID_REQUEST);
                assert!(e.message().contains("managed by the inbound router"));
            },
            _ => panic!("Expected Call error, got {err:?}"),
        }

        // 4. Attempt to save the connector session via RPC.
        let save_res = client.save_session(session_id.clone()).await;
        let err = save_res.expect_err("save_session on a connector session should fail");
        match err {
            JsonRpcError::Call(e) => {
                assert_eq!(e.code(), crate::rpc::error_codes::INVALID_REQUEST);
                assert!(e.message().contains("managed by the inbound router"));
            },
            _ => panic!("Expected Call error, got {err:?}"),
        }
    }
}
