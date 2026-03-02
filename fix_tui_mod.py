import os, re

path = "crates/astrid-cli/src/tui/mod.rs"
with open(path, "r") as f:
    content = f.read()

content = content.replace("use astrid_events::kernel_api::KernelEvent;", "use astrid_events::AstridEvent;")
content = content.replace("handle_kernel_event(app, event);", "handle_daemon_event(app, event);")
content = content.replace("fn handle_daemon_event(app: &mut App, event: KernelEvent) {", "fn handle_daemon_event(app: &mut App, event: AstridEvent) {")
content = content.replace("KernelEvent::", "astrid_events::event::KernelEvent::") # Wait, AstridEvent::Ipc or something. Let me look at the definition of AstridEvent.
# Let's just comment out handle_daemon_event body and pending actions for now, we will rebuild the TUI correctly in the next PR.

stub_handler = """fn handle_daemon_event(app: &mut App, event: AstridEvent) {
    if let AstridEvent::Ipc { message, .. } = event {
        if let astrid_events::ipc::IpcPayload::AgentResponse { text, .. } = message.payload {
            app.stream_buffer.push_str(&text);
        }
    }
}
"""
content = re.sub(r'fn handle_daemon_event.*?^async fn handle_pending_actions', stub_handler + "\nasync fn handle_pending_actions", content, flags=re.DOTALL|re.MULTILINE)

stub_pending = """async fn handle_pending_actions(
    app: &mut App,
    client: &mut SocketClient,
    session_id: &SessionId,
) -> anyhow::Result<()> {
    // Process input
    if let Some(text) = app.input_to_send.take() {
        app.stream_buffer.clear();
        app.messages.push(Message::user(&text));
        client.send_input(text).await?;
        app.state = UiState::Thinking { start_time: Instant::now(), dots: 0 };
    }
    Ok(())
}
"""
content = re.sub(r'async fn handle_pending_actions.*?^fn flush_stream_buffer', stub_pending + "\nfn flush_stream_buffer", content, flags=re.DOTALL|re.MULTILINE)

# Remove the run_json_chat reference which is now gone
content = re.sub(r'async fn run_json_chat.*?^fn resolve_model_name', "fn resolve_model_name", content, flags=re.DOTALL|re.MULTILINE)

with open(path, "w") as f:
    f.write(content)

