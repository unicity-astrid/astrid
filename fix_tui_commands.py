import os, re

path = "crates/astrid-cli/src/tui/mod.rs"
with open(path, "r") as f:
    content = f.read()

# I will just stub out handle_slash_command entirely for now to fix the compiler errors. 
# We need to rewrite these commands to use the new `kernel.request.*` IPC events anyway.

stub_slash = """async fn handle_slash_command(
    cmd: &str,
    app: &mut App,
    client: &mut SocketClient,
    session_id: &SessionId,
) {
    if cmd == "quit" || cmd == "exit" || cmd == "q" {
        app.should_quit = true;
    } else {
        app.push_notice(&format!("Command not implemented in microkernel UI: {}", cmd));
    }
}"""

content = re.sub(r'async fn handle_slash_command.*?^}$', stub_slash, content, flags=re.DOTALL|re.MULTILINE)
content = content.replace("client: &DaemonClient", "client: &mut SocketClient")

with open(path, "w") as f:
    f.write(content)

