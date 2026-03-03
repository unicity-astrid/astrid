import os, re

path = "crates/astrid-capsule/src/engine/wasm/mod.rs"
with open(path, "r") as f:
    content = f.read()

content = content.replace("cli_socket_listener: ctx.cli_socket_listener.clone(),", "cli_socket_listener: ctx.cli_socket_listener.clone(),")
if "cli_socket_listener" not in content:
    content = content.replace("config: wasm_config,", "config: wasm_config,\n                cli_socket_listener: ctx.cli_socket_listener.clone(),")

with open(path, "w") as f:
    f.write(content)

path_hs = "crates/astrid-capsule/src/engine/wasm/host_state.rs"
with open(path_hs, "r") as f:
    content = f.read()

content = content.replace("registered_connectors: Vec::new(),", "registered_connectors: Vec::new(),\n            cli_socket_listener: None,")

with open(path_hs, "w") as f:
    f.write(content)

