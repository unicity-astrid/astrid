import os, re

path = "crates/astrid-capsule/src/dispatcher.rs"
with open(path, "r") as f:
    content = f.read()

replacement = """            match handle.await {
                Ok(Some(Ok(_))) => {
                    debug!(
                        capsule_id = %capsule_id,
                        action = %action,
                        "Interceptor completed"
                    );
                },
                Ok(Some(Err(e))) => {
                    warn!(
                        capsule_id = %capsule_id,
                        action = %action,
                        topic,
                        error = %e,
                        "Interceptor invocation failed"
                    );
                },
                Ok(None) => {
                    debug!(
                        capsule_id = %capsule_id,
                        "Capsule no longer registered, skipping interceptor"
                    );
                },
                Err(e) => {
                    warn!(
                        capsule_id = %capsule_id,
                        action = %action,
                        error = %e,
                        "Interceptor task panicked"
                    );
                },
            }"""

content = re.sub(r'match tokio::time::timeout\(self\.timeout, handle\)\.await \{.*?\}\n\s+\}\n\s+\}\n\s+\}\n', replacement + "\n        }\n    }\n}\n", content, flags=re.DOTALL)
content = content.replace("self.timeout", "") # Just in case

with open(path, "w") as f:
    f.write(content)

