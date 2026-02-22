//! Secret management for the gateway.

use crate::error::{GatewayError, GatewayResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// Thread-safe secret storage and expansion.
#[derive(Clone, Default)]
pub struct Secrets {
    /// Secret key-value pairs behind a lock for thread safety.
    inner: Arc<RwLock<HashMap<String, String>>>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys = self.keys();
        f.debug_struct("Secrets")
            .field("count", &keys.len())
            .field("keys", &keys)
            .finish()
    }
}

/// Serialization helper — serializes only key names, not values.
impl Serialize for Secrets {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let keys = self.keys();
        keys.serialize(serializer)
    }
}

/// Deserialization helper — deserializes directly into the inner map.
impl<'de> Deserialize<'de> for Secrets {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = HashMap::<String, String>::deserialize(deserializer)?;
        Ok(Self {
            inner: Arc::new(RwLock::new(map)),
        })
    }
}

impl Secrets {
    /// Create an empty secrets store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load secrets from a TOML file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load<P: AsRef<Path>>(path: P) -> GatewayResult<Self> {
        let path = path.as_ref();
        let mut file = std::fs::File::open(path)?;

        // Check file permissions (should be 0600 or 0400)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = file.metadata()?;
            let mode = metadata.permissions().mode();
            if mode & 0o077 != 0 {
                return Err(GatewayError::Secret(format!(
                    "secrets file {} has insecure permissions {:o}, should be 0600",
                    path.display(),
                    mode & 0o777
                )));
            }
        }

        let mut contents = String::new();
        std::io::Read::read_to_string(&mut file, &mut contents)?;
        let secrets: HashMap<String, String> = toml::from_str(&contents)?;
        Ok(Self {
            inner: Arc::new(RwLock::new(secrets)),
        })
    }

    /// Get a secret by key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        let guard = self.inner.read().ok()?;
        guard.get(key).cloned()
    }

    /// Set a secret.
    pub fn set(&self, key: impl Into<String>, value: impl Into<String>) {
        if let Ok(mut guard) = self.inner.write() {
            guard.insert(key.into(), value.into());
        }
    }

    /// Check if a secret exists.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.inner
            .read()
            .ok()
            .is_some_and(|guard| guard.contains_key(key))
    }

    /// Expand a string with secret and environment variable references.
    ///
    /// Supports:
    /// - `${secrets.key}` - Expands to secret value
    /// - `${env:VAR}` - Expands to environment variable (explicit prefix)
    /// - `${VAR}` - Expands to environment variable (shorthand)
    /// - `${VAR:-default}` - Expands to env var or default if not set
    ///
    /// # Errors
    ///
    /// Returns an error if a referenced secret or required env var is missing.
    pub fn expand(&self, input: &str) -> GatewayResult<String> {
        self.expand_with_env(input, |var| std::env::var(var).ok())
    }

    /// Internal expander that takes a custom environment resolver for testing.
    fn expand_with_env<F>(&self, input: &str, env_resolver: F) -> GatewayResult<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut result = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '$' && chars.peek() == Some(&'{') {
                chars.next(); // consume '{'

                let mut var_name = String::new();
                let mut default_value = None;

                while let Some(&c) = chars.peek() {
                    if c == '}' {
                        chars.next();
                        break;
                    }
                    if c == ':' && chars.clone().nth(1) == Some('-') {
                        chars.next(); // consume ':'
                        chars.next(); // consume '-'
                        let mut default = String::new();
                        while let Some(&c) = chars.peek() {
                            if c == '}' {
                                break;
                            }
                            chars.next();
                            default.push(c);
                        }
                        default_value = Some(default);
                    } else {
                        chars.next();
                        var_name.push(c);
                    }
                }

                let value = if let Some(key) = var_name.strip_prefix("secrets.") {
                    // ${secrets.key} — resolve from secrets store
                    self.get(key)
                        .or(default_value)
                        .ok_or_else(|| GatewayError::Secret(format!("secret not found: {key}")))?
                } else if let Some(env_var) = var_name.strip_prefix("env:") {
                    // ${env:VAR} — explicit env var syntax
                    env_resolver(env_var).or(default_value).ok_or_else(|| {
                        GatewayError::Secret(format!("environment variable not set: {env_var}"))
                    })?
                } else {
                    // ${VAR} — shorthand env var syntax
                    env_resolver(&var_name).or(default_value).ok_or_else(|| {
                        GatewayError::Secret(format!("environment variable not set: {var_name}"))
                    })?
                };

                result.push_str(&value);
            } else {
                result.push(c);
            }
        }

        Ok(result)
    }

    /// List all secret keys (without values).
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        self.inner
            .read()
            .map_or_else(|_| Vec::new(), |guard| guard.keys().cloned().collect())
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_secrets_basic() {
        let secrets = Secrets::new();
        secrets.set("api_key", "secret123");

        assert!(secrets.contains("api_key"));
        assert_eq!(secrets.get("api_key"), Some("secret123".to_string()));
        assert_eq!(secrets.get("missing"), None);
    }

    #[test]
    fn test_expand_secrets() {
        let secrets = Secrets::new();
        secrets.set("api_key", "my-api-key");

        let result = secrets.expand("Bearer ${secrets.api_key}").unwrap();
        assert_eq!(result, "Bearer my-api-key");
    }

    #[test]
    fn test_expand_env_var() {
        let secrets = Secrets::new();
        let result = secrets
            .expand_with_env("Value: ${TEST_VAR_FOR_SECRETS}", |var| {
                if var == "TEST_VAR_FOR_SECRETS" {
                    Some("test_value".to_string())
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(result, "Value: test_value");
    }

    #[test]
    fn test_expand_env_prefix() {
        let secrets = Secrets::new();
        let result = secrets
            .expand_with_env("Value: ${env:TEST_ENV_PREFIX_VAR}", |var| {
                if var == "TEST_ENV_PREFIX_VAR" {
                    Some("prefixed_value".to_string())
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(result, "Value: prefixed_value");
    }

    #[test]
    fn test_expand_env_prefix_missing() {
        let secrets = Secrets::new();
        let result = secrets.expand("${env:NONEXISTENT_ENV_PREFIX_VAR}");
        assert!(result.is_err());
    }

    #[test]
    fn test_expand_with_default() {
        let secrets = Secrets::new();
        let result = secrets
            .expand("Value: ${NONEXISTENT_VAR:-default_value}")
            .unwrap();
        assert_eq!(result, "Value: default_value");
    }

    #[test]
    fn test_expand_missing_secret() {
        let secrets = Secrets::new();
        let result = secrets.expand("${secrets.missing}");
        assert!(result.is_err());
    }

    #[test]
    fn test_expand_mixed() {
        let secrets = Secrets::new();
        secrets.set("suffix", "suffix");

        let result = secrets
            .expand_with_env("${TEST_PREFIX}-${secrets.suffix}", |var| {
                if var == "TEST_PREFIX" {
                    Some("prefix".to_string())
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(result, "prefix-suffix");
    }

    #[test]
    fn test_keys() {
        let secrets = Secrets::new();
        secrets.set("key1", "value1");
        secrets.set("key2", "value2");

        let keys = secrets.keys();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"key1".to_string()));
        assert!(keys.contains(&"key2".to_string()));
    }

    #[test]
    fn test_thread_safety() {
        let secrets = Secrets::new();
        let secrets_clone = secrets.clone();

        let handle = std::thread::spawn(move || {
            secrets_clone.set("from_thread", "thread_value");
        });

        handle.join().unwrap();
        assert_eq!(secrets.get("from_thread"), Some("thread_value".to_string()));
    }
}
