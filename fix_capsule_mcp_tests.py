import os, re

path = "crates/astrid-capsule/src/engine/mcp_tests.rs"
with open(path, "r") as f:
    content = f.read()

content = content.replace("event_bus,\n        };", "event_bus,\n            cli_socket_listener: None,\n        };")

with open(path, "w") as f:
    f.write(content)

