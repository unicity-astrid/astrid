//! npm package specifier parsing and validation.
//!
//! Parses specifiers like `@scope/name@version`, `name@version`, `@scope/name`, `name`.
//! Validates names against npm naming rules.

use crate::error::{PluginError, PluginResult};

/// Maximum npm package name length (scope + name combined).
const MAX_PACKAGE_NAME_LENGTH: usize = 214;

/// A parsed npm package specifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmSpec {
    /// Optional scope (without the `@` prefix).
    pub scope: Option<String>,
    /// Package name (without scope).
    pub name: String,
    /// Optional version or dist-tag (e.g. `"1.0.0"`, `"latest"`).
    pub version: Option<String>,
}

impl NpmSpec {
    /// Parse an npm package specifier string.
    ///
    /// Accepted formats:
    /// - `@scope/name@version`
    /// - `@scope/name`
    /// - `name@version`
    /// - `name`
    ///
    /// Validates scope and name against npm naming rules:
    /// - Must match `[a-z0-9][a-z0-9._-]*`
    /// - Combined length must not exceed 214 characters
    ///
    /// # Errors
    ///
    /// Returns `PluginError::RegistryError` if the specifier is empty or malformed.
    /// Returns `PluginError::InvalidPackageName` if the name fails validation.
    #[allow(clippy::arithmetic_side_effects)] // index arithmetic on find() results is safe
    pub fn parse(spec: &str) -> PluginResult<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Err(PluginError::RegistryError {
                message: "empty package specifier".into(),
            });
        }

        let parsed = if let Some(without_at) = spec.strip_prefix('@') {
            // Scoped package: @scope/name or @scope/name@version
            let slash_pos = without_at
                .find('/')
                .ok_or_else(|| PluginError::RegistryError {
                    message: format!("invalid scoped package specifier: {spec}"),
                })?;

            let scope = &without_at[..slash_pos];

            // Reject scopes that start with @ (e.g. @@foo/bar).
            if scope.starts_with('@') || scope.is_empty() {
                return Err(PluginError::RegistryError {
                    message: format!("invalid scoped package specifier: {spec}"),
                });
            }
            let rest = &without_at[slash_pos + 1..];

            if rest.is_empty() {
                return Err(PluginError::RegistryError {
                    message: format!("invalid scoped package specifier: {spec}"),
                });
            }

            // Check for version after the name
            if let Some(at_pos) = rest.find('@') {
                let name = &rest[..at_pos];
                let version = &rest[at_pos + 1..];
                if name.is_empty() || version.is_empty() {
                    return Err(PluginError::RegistryError {
                        message: format!("invalid scoped package specifier: {spec}"),
                    });
                }
                Self {
                    scope: Some(scope.to_string()),
                    name: name.to_string(),
                    version: Some(version.to_string()),
                }
            } else {
                Self {
                    scope: Some(scope.to_string()),
                    name: rest.to_string(),
                    version: None,
                }
            }
        } else {
            // Unscoped package: name or name@version
            if let Some(at_pos) = spec.find('@') {
                let name = &spec[..at_pos];
                let version = &spec[at_pos + 1..];
                if name.is_empty() || version.is_empty() {
                    return Err(PluginError::RegistryError {
                        message: format!("invalid package specifier: {spec}"),
                    });
                }
                Self {
                    scope: None,
                    name: name.to_string(),
                    version: Some(version.to_string()),
                }
            } else {
                Self {
                    scope: None,
                    name: spec.to_string(),
                    version: None,
                }
            }
        };

        // Validate names
        parsed.validate()?;
        Ok(parsed)
    }

    /// Validate the scope and name against npm naming rules.
    fn validate(&self) -> PluginResult<()> {
        // Check total length
        let full_name = self.full_name();
        if full_name.len() > MAX_PACKAGE_NAME_LENGTH {
            return Err(PluginError::InvalidPackageName {
                name: full_name,
                reason: format!("exceeds maximum length of {MAX_PACKAGE_NAME_LENGTH} characters"),
            });
        }

        // Validate scope if present
        if let Some(scope) = &self.scope {
            validate_name_component(scope, "scope")?;
        }

        // Validate name
        validate_name_component(&self.name, "name")?;

        Ok(())
    }

    /// Full package name including scope (e.g. `@scope/name` or `name`).
    #[must_use]
    pub fn full_name(&self) -> String {
        match &self.scope {
            Some(scope) => format!("@{scope}/{}", self.name),
            None => self.name.clone(),
        }
    }

    /// URL-encoded registry path for API calls.
    ///
    /// Scoped packages use `@scope%2Fname`, unscoped use `name`.
    /// Components are percent-encoded as defense-in-depth after validation.
    #[must_use]
    pub fn registry_path(&self) -> String {
        match &self.scope {
            Some(scope) => format!(
                "@{}%2F{}",
                percent_encode(scope),
                percent_encode(&self.name)
            ),
            None => percent_encode(&self.name),
        }
    }
}

/// Validate a single name component (scope or package name) against npm rules.
///
/// Must match `[a-z0-9][a-z0-9._-]*` (lowercase only, starts with alphanumeric).
fn validate_name_component(name: &str, kind: &str) -> PluginResult<()> {
    if name.is_empty() {
        return Err(PluginError::InvalidPackageName {
            name: name.to_string(),
            reason: format!("{kind} cannot be empty"),
        });
    }

    let mut chars = name.chars();
    let first = chars.next().expect("checked is_empty above");

    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(PluginError::InvalidPackageName {
            name: name.to_string(),
            reason: format!("{kind} must start with a lowercase letter or digit"),
        });
    }

    for c in chars {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '.' && c != '_' && c != '-' {
            return Err(PluginError::InvalidPackageName {
                name: name.to_string(),
                reason: format!(
                    "{kind} contains invalid character '{c}' (allowed: a-z, 0-9, '.', '_', '-')"
                ),
            });
        }
    }

    Ok(())
}

/// Percent-encode a URL path component (defense-in-depth after validation).
fn percent_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

impl std::fmt::Display for NpmSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.full_name())?;
        if let Some(version) = &self.version {
            write!(f, "@{version}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_scoped_with_version() {
        let spec = NpmSpec::parse("@openclaw/hello-tool@1.0.0").unwrap();
        assert_eq!(spec.scope.as_deref(), Some("openclaw"));
        assert_eq!(spec.name, "hello-tool");
        assert_eq!(spec.version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn parse_scoped_without_version() {
        let spec = NpmSpec::parse("@openclaw/hello-tool").unwrap();
        assert_eq!(spec.scope.as_deref(), Some("openclaw"));
        assert_eq!(spec.name, "hello-tool");
        assert_eq!(spec.version, None);
    }

    #[test]
    fn parse_unscoped_with_version() {
        let spec = NpmSpec::parse("hello-tool@2.3.4").unwrap();
        assert_eq!(spec.scope, None);
        assert_eq!(spec.name, "hello-tool");
        assert_eq!(spec.version.as_deref(), Some("2.3.4"));
    }

    #[test]
    fn parse_unscoped_without_version() {
        let spec = NpmSpec::parse("hello-tool").unwrap();
        assert_eq!(spec.scope, None);
        assert_eq!(spec.name, "hello-tool");
        assert_eq!(spec.version, None);
    }

    #[test]
    fn parse_unscoped_with_latest() {
        let spec = NpmSpec::parse("hello-tool@latest").unwrap();
        assert_eq!(spec.version.as_deref(), Some("latest"));
    }

    #[test]
    fn parse_empty_fails() {
        assert!(NpmSpec::parse("").is_err());
    }

    #[test]
    fn parse_bare_at_fails() {
        assert!(NpmSpec::parse("@").is_err());
    }

    #[test]
    fn parse_double_at_fails() {
        assert!(NpmSpec::parse("@@foo/bar").is_err());
    }

    #[test]
    fn parse_missing_name_after_scope_fails() {
        assert!(NpmSpec::parse("@scope/").is_err());
    }

    #[test]
    fn full_name_scoped() {
        let spec = NpmSpec::parse("@openclaw/hello-tool@1.0.0").unwrap();
        assert_eq!(spec.full_name(), "@openclaw/hello-tool");
    }

    #[test]
    fn full_name_unscoped() {
        let spec = NpmSpec::parse("hello-tool").unwrap();
        assert_eq!(spec.full_name(), "hello-tool");
    }

    #[test]
    fn registry_path_scoped() {
        let spec = NpmSpec::parse("@openclaw/hello-tool").unwrap();
        assert_eq!(spec.registry_path(), "@openclaw%2Fhello-tool");
    }

    #[test]
    fn registry_path_unscoped() {
        let spec = NpmSpec::parse("hello-tool").unwrap();
        assert_eq!(spec.registry_path(), "hello-tool");
    }

    #[test]
    fn display_full() {
        let spec = NpmSpec::parse("@openclaw/hello-tool@1.0.0").unwrap();
        assert_eq!(spec.to_string(), "@openclaw/hello-tool@1.0.0");
    }

    #[test]
    fn display_no_version() {
        let spec = NpmSpec::parse("@openclaw/hello-tool").unwrap();
        assert_eq!(spec.to_string(), "@openclaw/hello-tool");
    }

    #[test]
    fn whitespace_trimmed() {
        let spec = NpmSpec::parse("  hello-tool@1.0.0  ").unwrap();
        assert_eq!(spec.name, "hello-tool");
        assert_eq!(spec.version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn reject_uppercase_name() {
        let err = NpmSpec::parse("Hello-Tool").unwrap_err();
        assert!(err.to_string().contains("invalid package name"));
    }

    #[test]
    fn reject_name_starting_with_dot() {
        let err = NpmSpec::parse(".hidden-pkg").unwrap_err();
        assert!(err.to_string().contains("invalid package name"));
    }

    #[test]
    fn reject_name_with_special_chars() {
        let err = NpmSpec::parse("pkg/../../etc/passwd").unwrap_err();
        assert!(err.to_string().contains("invalid"));
    }

    #[test]
    fn reject_overly_long_name() {
        let long_name = "a".repeat(MAX_PACKAGE_NAME_LENGTH + 1);
        let err = NpmSpec::parse(&long_name).unwrap_err();
        assert!(err.to_string().contains("maximum length"));
    }

    #[test]
    fn valid_names_accepted() {
        NpmSpec::parse("my-pkg").unwrap();
        NpmSpec::parse("my_pkg").unwrap();
        NpmSpec::parse("my.pkg").unwrap();
        NpmSpec::parse("0my-pkg").unwrap();
        NpmSpec::parse("@my-scope/my-pkg").unwrap();
    }
}
