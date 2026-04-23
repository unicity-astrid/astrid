//! Static group-to-capability configuration (Layer 5, issue #670).
//!
//! A [`GroupConfig`] names a small set of built-in groups
//! ([`BUILTIN_ADMIN`], [`BUILTIN_AGENT`], [`BUILTIN_RESTRICTED`]) and
//! optionally merges operator-defined custom groups from
//! `$ASTRID_HOME/etc/groups.toml`. Each group confers a set of capability
//! patterns, evaluated left-to-right against the colon-delimited Layer 5
//! grammar in [`crate::capability_grammar`].
//!
//! # Design contract
//!
//! - Built-in groups are baked in. Attempting to redefine them in
//!   `groups.toml` is a hard error at load time.
//! - Custom groups go through [`validate_capability`] for every entry.
//! - The universal `*` pattern is reserved for the built-in `admin`
//!   group. Custom groups may grant it only by explicitly opting in via
//!   `unsafe_admin = true` on that group; otherwise it's rejected at load.
//! - Missing `groups.toml` → built-ins only (the single-tenant default).
//! - Malformed TOML, unknown fields, or duplicate group names are hard
//!   errors — this fails the kernel boot, which is intentional.
//! - `GroupConfig::get` returning `None` for a name referenced by a
//!   principal profile is **not** an error here; the caller (Layer 5
//!   [`CapabilityCheck`](../../../astrid-capabilities/src/policy.rs))
//!   treats it as fail-closed and logs a `warn!`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::capability_grammar::{CapabilityGrammarError, validate_capability};
use crate::dirs::AstridHome;

/// Canonical name of the built-in administrator group.
pub const BUILTIN_ADMIN: &str = "admin";
/// Canonical name of the built-in agent group (self-scoped capabilities).
pub const BUILTIN_AGENT: &str = "agent";
/// Canonical name of the built-in restricted group (no capabilities).
pub const BUILTIN_RESTRICTED: &str = "restricted";

const BUILTIN_NAMES: [&str; 3] = [BUILTIN_ADMIN, BUILTIN_AGENT, BUILTIN_RESTRICTED];

/// Errors raised when loading or validating a [`GroupConfig`].
#[derive(Debug, Error)]
pub enum GroupConfigError {
    /// Filesystem IO failed while reading `groups.toml`.
    #[error("groups config io error: {0}")]
    Io(#[from] io::Error),
    /// `groups.toml` failed to parse as TOML, or contains unknown fields.
    #[error("groups config parse error: {0}")]
    Parse(#[from] toml::de::Error),
    /// A group entry attempts to redefine a reserved built-in group name.
    #[error("built-in group {name:?} may not be redefined in groups.toml")]
    RedefinedBuiltin {
        /// Name of the built-in group the config tried to overwrite.
        name: String,
    },
    /// Two different group entries share the same name.
    ///
    /// Note: the TOML parser itself rejects duplicate keys in a single
    /// table, but this variant covers the future case where multiple
    /// sources are merged.
    #[error("groups config declares {name:?} more than once")]
    DuplicateName {
        /// Duplicated group name.
        name: String,
    },
    /// A group entry contains a capability that fails the Layer 5
    /// grammar validator.
    #[error("groups config: group {group:?} capability {cap:?} rejected: {reason}")]
    InvalidCapability {
        /// Name of the offending group.
        group: String,
        /// Raw capability string that failed validation.
        cap: String,
        /// Underlying grammar error.
        reason: CapabilityGrammarError,
    },
    /// A custom group grants the universal `*` capability without
    /// opting in to the `unsafe_admin` flag.
    #[error(
        "groups config: custom group {group:?} grants '*' (universal admin); \
         set `unsafe_admin = true` to confirm this elevation"
    )]
    UnsafeUniversalGrant {
        /// Name of the offending custom group.
        group: String,
    },
}

/// Result alias for [`GroupConfig`] operations.
pub type GroupConfigResult<T> = Result<T, GroupConfigError>;

/// A named set of capability patterns.
///
/// Custom groups can opt-in to granting the universal `*` capability by
/// setting [`Group::unsafe_admin`] — intended as a safeguard against
/// typo-driven privilege escalation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    /// Capability patterns this group confers. Each pattern is validated
    /// by [`validate_capability`] at load time.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Human-readable description surfaced in CLI and audit log views.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Opt-in flag: custom groups must set this to grant the universal
    /// `*` capability, making the elevation deliberate and visible in
    /// the config.
    #[serde(default)]
    pub unsafe_admin: bool,
}

/// The frozen group-to-capability map consumed by
/// [`CapabilityCheck`](../../../astrid-capabilities/src/policy.rs).
///
/// Built at kernel boot from built-ins merged with any operator-provided
/// `groups.toml`. Treat the resulting value as immutable — hot reload is
/// deferred to Layer 6.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupConfig {
    /// Group name → group definition.
    pub groups: HashMap<String, Group>,
}

/// TOML wrapper: the on-disk representation uses a top-level `[groups.*]`
/// table so operators write `[groups.ops]` rather than a nested
/// `[[groups]]` array.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupsFile {
    #[serde(default)]
    groups: HashMap<String, Group>,
}

impl GroupConfig {
    /// Canonical on-disk path for the system-wide groups config.
    #[must_use]
    pub fn path_for(home: &AstridHome) -> PathBuf {
        home.etc_dir().join("groups.toml")
    }

    /// Return a [`GroupConfig`] containing only the built-in groups.
    #[must_use]
    pub fn builtin_only() -> Self {
        let mut groups = HashMap::with_capacity(BUILTIN_NAMES.len());
        for (name, group) in builtin_entries() {
            groups.insert(name.to_string(), group);
        }
        Self { groups }
    }

    /// Load the group config from `home`'s `etc/groups.toml`, falling
    /// back to [`Self::builtin_only`] if the file is absent.
    ///
    /// # Errors
    ///
    /// See [`GroupConfigError`].
    pub fn load(home: &AstridHome) -> GroupConfigResult<Self> {
        Self::load_from_path(&Self::path_for(home))
    }

    /// Load the group config from an explicit path.
    ///
    /// # Errors
    ///
    /// See [`GroupConfigError`].
    pub fn load_from_path(path: &Path) -> GroupConfigResult<Self> {
        let contents = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::builtin_only());
            },
            Err(e) => return Err(GroupConfigError::Io(e)),
        };
        Self::from_toml_str(&contents)
    }

    /// Parse a [`GroupConfig`] from raw TOML, merging with the built-ins.
    ///
    /// # Errors
    ///
    /// See [`GroupConfigError`].
    pub fn from_toml_str(contents: &str) -> GroupConfigResult<Self> {
        let file: GroupsFile = toml::from_str(contents)?;
        Self::from_custom_groups(file.groups)
    }

    fn from_custom_groups(custom: HashMap<String, Group>) -> GroupConfigResult<Self> {
        // Reject redefinition of any built-in group.
        for name in custom.keys() {
            if is_builtin(name) {
                return Err(GroupConfigError::RedefinedBuiltin { name: name.clone() });
            }
        }

        // Validate each custom group's capability entries.
        let mut seen: HashSet<&str> = HashSet::new();
        for (name, group) in &custom {
            if !seen.insert(name.as_str()) {
                return Err(GroupConfigError::DuplicateName { name: name.clone() });
            }
            validate_custom_group(name, group)?;
        }

        let mut groups = HashMap::with_capacity(BUILTIN_NAMES.len().saturating_add(custom.len()));
        for (name, group) in builtin_entries() {
            groups.insert(name.to_string(), group);
        }
        for (name, group) in custom {
            groups.insert(name, group);
        }

        Ok(Self { groups })
    }

    /// Look up a group by name, if present.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Group> {
        self.groups.get(name)
    }

    /// Number of groups in the resolved config (built-ins + custom).
    #[must_use]
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Whether the config contains no groups. Always `false` in practice
    /// because built-ins are baked in.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Iterator over `(group_name, &Group)`.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Group)> {
        self.groups.iter()
    }
}

impl Default for GroupConfig {
    fn default() -> Self {
        Self::builtin_only()
    }
}

fn is_builtin(name: &str) -> bool {
    BUILTIN_NAMES.contains(&name)
}

fn builtin_entries() -> [(&'static str, Group); 3] {
    [
        (
            BUILTIN_ADMIN,
            Group {
                capabilities: vec!["*".to_string()],
                description: Some("Built-in administrator — universal capability grant".into()),
                // `admin` is the one group the universal `*` is reserved for;
                // the `unsafe_admin` flag is a custom-group concern.
                unsafe_admin: false,
            },
        ),
        (
            BUILTIN_AGENT,
            Group {
                capabilities: vec!["self:*".to_string(), "delegate:self:*".to_string()],
                description: Some(
                    "Built-in agent — self-scoped capability grants for routine agent workflows"
                        .into(),
                ),
                unsafe_admin: false,
            },
        ),
        (
            BUILTIN_RESTRICTED,
            Group {
                capabilities: Vec::new(),
                description: Some(
                    "Built-in restricted — no implicit capabilities; grants must be explicit"
                        .into(),
                ),
                unsafe_admin: false,
            },
        ),
    ]
}

fn validate_custom_group(name: &str, group: &Group) -> GroupConfigResult<()> {
    for cap in &group.capabilities {
        if let Err(reason) = validate_capability(cap) {
            return Err(GroupConfigError::InvalidCapability {
                group: name.to_string(),
                cap: cap.clone(),
                reason,
            });
        }
        if cap == "*" && !group.unsafe_admin {
            return Err(GroupConfigError::UnsafeUniversalGrant {
                group: name.to_string(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    #[test]
    fn builtin_only_contains_admin_agent_restricted() {
        let cfg = GroupConfig::builtin_only();
        assert_eq!(cfg.len(), 3);
        assert_eq!(
            cfg.get(BUILTIN_ADMIN).unwrap().capabilities,
            vec!["*".to_string()]
        );
        assert_eq!(
            cfg.get(BUILTIN_AGENT).unwrap().capabilities,
            vec!["self:*".to_string(), "delegate:self:*".to_string()]
        );
        assert!(cfg.get(BUILTIN_RESTRICTED).unwrap().capabilities.is_empty());
    }

    #[test]
    fn load_missing_file_returns_builtins() {
        let dir = tempdir().unwrap();
        let home = AstridHome::from_path(dir.path());
        assert!(!GroupConfig::path_for(&home).exists());
        let cfg = GroupConfig::load(&home).unwrap();
        assert_eq!(cfg.len(), 3);
    }

    #[test]
    fn load_merges_custom_groups_with_builtins() {
        let toml_doc = r#"
            [groups.ops]
            description = "Deployment operators"
            capabilities = ["capsule:install", "capsule:remove"]

            [groups.auditor]
            capabilities = ["audit:read", "agent:list"]
        "#;
        let cfg = GroupConfig::from_toml_str(toml_doc).unwrap();
        assert_eq!(cfg.len(), 5);
        assert_eq!(
            cfg.get("ops").unwrap().capabilities,
            vec!["capsule:install".to_string(), "capsule:remove".to_string()]
        );
        assert_eq!(cfg.get("auditor").unwrap().capabilities.len(), 2);
        // Built-ins remain intact.
        assert_eq!(
            cfg.get(BUILTIN_ADMIN).unwrap().capabilities,
            vec!["*".to_string()]
        );
    }

    #[test]
    fn rejects_redefined_builtin() {
        let toml_doc = r#"
            [groups.admin]
            capabilities = ["custom:garbage"]
        "#;
        let err = GroupConfig::from_toml_str(toml_doc).unwrap_err();
        match err {
            GroupConfigError::RedefinedBuiltin { name } => assert_eq!(name, BUILTIN_ADMIN),
            other => panic!("expected RedefinedBuiltin, got: {other:?}"),
        }
    }

    #[test]
    fn rejects_redefined_agent_builtin() {
        let toml_doc = r#"
            [groups.agent]
            capabilities = ["self:capsule:install"]
        "#;
        assert!(matches!(
            GroupConfig::from_toml_str(toml_doc),
            Err(GroupConfigError::RedefinedBuiltin { .. })
        ));
    }

    #[test]
    fn rejects_unknown_top_level_field() {
        let toml_doc = r#"
            typo_field = true
            [groups.ops]
            capabilities = ["capsule:install"]
        "#;
        assert!(matches!(
            GroupConfig::from_toml_str(toml_doc),
            Err(GroupConfigError::Parse(_))
        ));
    }

    #[test]
    fn rejects_unknown_group_field() {
        let toml_doc = r#"
            [groups.ops]
            priviledges = []
        "#;
        assert!(matches!(
            GroupConfig::from_toml_str(toml_doc),
            Err(GroupConfigError::Parse(_))
        ));
    }

    #[test]
    fn rejects_invalid_capability_grammar() {
        let toml_doc = r#"
            [groups.ops]
            capabilities = ["system:shut down"]
        "#;
        let err = GroupConfig::from_toml_str(toml_doc).unwrap_err();
        match err {
            GroupConfigError::InvalidCapability { group, cap, .. } => {
                assert_eq!(group, "ops");
                assert_eq!(cap, "system:shut down");
            },
            other => panic!("expected InvalidCapability, got: {other:?}"),
        }
    }

    #[test]
    fn rejects_custom_group_with_universal_star_without_opt_in() {
        let toml_doc = r#"
            [groups.privileged]
            capabilities = ["*"]
        "#;
        let err = GroupConfig::from_toml_str(toml_doc).unwrap_err();
        assert!(matches!(err, GroupConfigError::UnsafeUniversalGrant { .. }));
    }

    #[test]
    fn accepts_custom_group_with_universal_star_and_opt_in() {
        let toml_doc = r#"
            [groups.privileged]
            unsafe_admin = true
            capabilities = ["*"]
        "#;
        let cfg = GroupConfig::from_toml_str(toml_doc).unwrap();
        assert_eq!(
            cfg.get("privileged").unwrap().capabilities,
            vec!["*".to_string()]
        );
        assert!(cfg.get("privileged").unwrap().unsafe_admin);
    }

    #[test]
    fn rejects_double_glob_capability() {
        let toml_doc = r#"
            [groups.ops]
            capabilities = ["capsule:**"]
        "#;
        let err = GroupConfig::from_toml_str(toml_doc).unwrap_err();
        match err {
            GroupConfigError::InvalidCapability { reason, .. } => {
                assert_eq!(reason, CapabilityGrammarError::DoubleStar);
            },
            other => panic!("expected InvalidCapability(DoubleStar), got: {other:?}"),
        }
    }

    #[test]
    fn load_from_path_parses_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("groups.toml");
        fs::write(
            &path,
            "[groups.ops]\ncapabilities = [\"capsule:install\"]\n",
        )
        .unwrap();
        let cfg = GroupConfig::load_from_path(&path).unwrap();
        assert!(cfg.get("ops").is_some());
    }

    #[test]
    fn get_returns_none_for_unknown_name() {
        let cfg = GroupConfig::builtin_only();
        assert!(cfg.get("not-a-real-group").is_none());
    }
}
