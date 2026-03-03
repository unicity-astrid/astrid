import os, re

path = "crates/astrid-capsule/src/engine/wasm/host_state.rs"
with open(path, "r") as f:
    content = f.read()

content = content.replace("registered_connectors: Vec::new(),", "registered_connectors: Vec::new(),\n            cli_socket_listener: None,")

with open(path, "w") as f:
    f.write(content)

