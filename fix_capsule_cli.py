import os, re

path = "crates/astrid-capsule-cli/src/lib.rs"
with open(path, "r") as f:
    content = f.read()

# I see. In astrid-sdk-macros, #[capsule] is for the impl block, not for main functions!
# For main functions, we just use #[plugin_fn] from extism_pdk

content = content.replace("#[capsule::main]", "#[plugin_fn]")

with open(path, "w") as f:
    f.write(content)

