//! Distro manifest validation.
//!
//! Validates structural constraints that TOML deserialization alone cannot
//! enforce: schema version, identifier formats, semver, variable references,
//! role presence, and duplicate names.

use std::collections::HashSet;

use super::manifest::{DistroManifest, SCHEMA_VERSION};

/// Check if a string is a valid identifier: `^[a-z][a-z0-9-]*$`.
fn is_valid_id(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Extract `{{ var_name }}` references from a template string.
fn extract_variable_refs(template: &str) -> Vec<&str> {
    template
        .split("{{")
        .skip(1)
        .filter_map(|s| s.split_once("}}"))
        .map(|(var, _)| var.trim())
        .filter(|var| !var.is_empty())
        .collect()
}

/// Validate a parsed distro manifest.
///
/// Checks that cannot be expressed in serde alone:
/// - Schema version is supported
/// - Distro ID format
/// - Distro version is valid semver
/// - astrid-version (if set) is valid semver requirement
/// - No duplicate capsule names
/// - At least one capsule
/// - At least one capsule with role = "uplink"
/// - Variable references in capsule env resolve to defined variables
/// - Requires version strings are valid semver requirements
pub(crate) fn validate_manifest(manifest: &DistroManifest) -> anyhow::Result<()> {
    // Schema version.
    if manifest.schema_version != SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported schema-version {} (expected {SCHEMA_VERSION})",
            manifest.schema_version,
        );
    }

    // Distro ID format.
    if !is_valid_id(&manifest.distro.id) {
        anyhow::bail!(
            "distro.id '{}' is invalid (must match ^[a-z][a-z0-9-]*$)",
            manifest.distro.id,
        );
    }

    // Distro version is valid semver.
    if semver::Version::parse(&manifest.distro.version).is_err() {
        anyhow::bail!(
            "distro.version '{}' is not valid semver",
            manifest.distro.version,
        );
    }

    // astrid-version is valid semver requirement (if set).
    if let Some(ref av) = manifest.distro.astrid_version
        && semver::VersionReq::parse(av).is_err()
    {
        anyhow::bail!("distro.astrid-version '{av}' is not a valid semver requirement");
    }

    // Requires version strings are valid semver requirements.
    for (ns, ifaces) in &manifest.distro.requires {
        for (name, req) in ifaces {
            if semver::VersionReq::parse(req).is_err() {
                anyhow::bail!(
                    "distro.requires.{ns}.{name} '{req}' is not a valid semver requirement",
                );
            }
        }
    }

    // At least one capsule.
    if manifest.capsules.is_empty() {
        anyhow::bail!("distro must contain at least one capsule");
    }

    // No duplicate capsule names.
    let mut seen_names = HashSet::new();
    for cap in &manifest.capsules {
        if !seen_names.insert(&cap.name) {
            anyhow::bail!("duplicate capsule name '{}'", cap.name);
        }
    }

    // At least one uplink.
    let has_uplink = manifest
        .capsules
        .iter()
        .any(|c| c.role.as_deref() == Some("uplink"));
    if !has_uplink {
        anyhow::bail!("distro must have at least one capsule with role = \"uplink\" (a frontend)");
    }

    // Variable references in capsule env.
    let defined_vars: HashSet<&str> = manifest.variables.keys().map(String::as_str).collect();
    for cap in &manifest.capsules {
        for (key, value) in &cap.env {
            for var_ref in extract_variable_refs(value) {
                if !defined_vars.contains(var_ref) {
                    anyhow::bail!(
                        "capsule '{}' env.{key} references undefined variable '{{{{ {var_ref} }}}}'",
                        cap.name,
                    );
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_id_accepts_lowercase() {
        assert!(is_valid_id("astralis"));
        assert!(is_valid_id("my-distro"));
        assert!(is_valid_id("a1b2c3"));
    }

    #[test]
    fn is_valid_id_rejects_invalid() {
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("UPPER"));
        assert!(!is_valid_id("1starts-with-digit"));
        assert!(!is_valid_id("has space"));
        assert!(!is_valid_id("under_score"));
    }

    #[test]
    fn extract_refs_finds_variables() {
        assert_eq!(extract_variable_refs("{{ foo }}"), vec!["foo"]);
        assert_eq!(
            extract_variable_refs("prefix-{{ bar }}-{{ baz }}-suffix"),
            vec!["bar", "baz"]
        );
        assert_eq!(extract_variable_refs("no refs here"), Vec::<&str>::new());
        assert_eq!(extract_variable_refs("{{}}"), Vec::<&str>::new());
    }

    #[test]
    fn extract_refs_handles_whitespace() {
        assert_eq!(extract_variable_refs("{{  spaced  }}"), vec!["spaced"]);
        assert_eq!(extract_variable_refs("{{no_space}}"), vec!["no_space"]);
    }
}
