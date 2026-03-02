import os, re

path = "crates/astrid-cli/src/tui/mod.rs"
with open(path, "r") as f:
    content = f.read()

stub_pending = """async fn handle_pending_actions(
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
    
    // Approvals and elicitation will be translated to KernelRequest events in the next pass.
    app.pending_actions.clear();
    
    Ok(())
}
"""
content = re.sub(r'async fn handle_pending_actions.*?^fn flush_stream_buffer', stub_pending + "\nfn flush_stream_buffer", content, flags=re.DOTALL|re.MULTILINE)

with open(path, "w") as f:
    f.write(content)
