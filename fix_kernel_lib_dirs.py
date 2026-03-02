import os, re

path = "crates/astrid-kernel/src/lib.rs"
with open(path, "r") as f:
    content = f.read()

content = content.replace("home.capsules_dir()", "home.plugins_dir()")

with open(path, "w") as f:
    f.write(content)

