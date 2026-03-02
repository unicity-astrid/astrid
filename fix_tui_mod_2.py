import os, re

path = "crates/astrid-cli/src/tui/mod.rs"
with open(path, "r") as f:
    content = f.read()

# Replace the broken methods that rely on the old jsonrpsee daemon_client API
# We just need to replace the block inside `handle_pending_actions`
new_pending = """async fn handle_pending_actions(
    app: &mut App,
    client: &mut SocketClient,
    session_id: &SessionId,
) -> anyhow::Result<()> {
    // Process input
    if let Some(text) = app.input_to_send.take() {
        app.stream_buffer.clear();
        if text.starts_with('/') {
            handle_slash_command(&text, app, client, session_id).await;
        } else {
            app.messages.push(Message::user(&text));
            client.send_input(text).await?;
            app.state = UiState::Thinking { start_time: Instant::now(), dots: 0 };
        }
    }
    
    // Clear pending actions for now, we will handle Approvals via KernelRequest later
    app.pending_actions.clear();

    Ok(())
}
"""

content = re.sub(r'async fn handle_pending_actions.*?^fn flush_stream_buffer', new_pending + "\nfn flush_stream_buffer", content, flags=re.DOTALL|re.MULTILINE)

with open(path, "w") as f:
    f.write(content)

