//! Version management for state and configuration migrations.
//!
//! This module provides versioning primitives to ensure safe migrations
//! when stored data formats change between releases.

use std::fmt;
use std::num::ParseIntError;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Semantic version following semver conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Version {
    /// Major version - breaking changes
    pub major: u32,
    /// Minor version - new features, backwards compatible
    pub minor: u32,
    /// Patch version - bug fixes, backwards compatible
    pub patch: u32,
}

impl Version {
    /// Creates a new version.
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Returns the current crate version.
    #[must_use]
    pub fn current() -> Self {
        Self::new(
            env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap_or(0),
            env!("CARGO_PKG_VERSION_MINOR").parse().unwrap_or(1),
            env!("CARGO_PKG_VERSION_PATCH").parse().unwrap_or(0),
        )
    }

    /// Checks if this version is compatible with another version.
    ///
    /// Compatibility rules (SemVer):
    /// - Major version must match
    /// - Minor version of `other` must be >= self.minor
    #[must_use]
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.major == other.major && other.minor >= self.minor
    }

    /// Checks if this version is newer than another.
    #[must_use]
    pub fn is_newer_than(&self, other: &Self) -> bool {
        match self.major.cmp(&other.major) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => match self.minor.cmp(&other.minor) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Less => false,
                std::cmp::Ordering::Equal => self.patch > other.patch,
            },
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Default for Version {
    fn default() -> Self {
        Self::current()
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.major.cmp(&other.major) {
            std::cmp::Ordering::Equal => match self.minor.cmp(&other.minor) {
                std::cmp::Ordering::Equal => self.patch.cmp(&other.patch),
                ord => ord,
            },
            ord => ord,
        }
    }
}

/// Error returned when parsing a version string fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionParseError {
    /// Wrong number of segments (expected "major.minor.patch").
    InvalidFormat(String),
    /// A numeric segment could not be parsed.
    InvalidNumber(ParseIntError),
}

impl fmt::Display for VersionParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat(s) => write!(
                f,
                "invalid version format: {s} (expected major.minor.patch)"
            ),
            Self::InvalidNumber(e) => write!(f, "invalid version number: {e}"),
        }
    }
}

impl std::error::Error for VersionParseError {}

impl From<ParseIntError> for VersionParseError {
    fn from(e: ParseIntError) -> Self {
        Self::InvalidNumber(e)
    }
}

impl FromStr for Version {
    type Err = VersionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.trim().split('.').collect();
        if parts.len() != 3 {
            return Err(VersionParseError::InvalidFormat(s.to_string()));
        }
        Ok(Self {
            major: parts[0].parse()?,
            minor: parts[1].parse()?,
            patch: parts[2].parse()?,
        })
    }
}

impl Version {
    /// Parse a version from a string like "1.2.3".
    ///
    /// # Errors
    ///
    /// Returns an error if the string is not in "major.minor.patch" format.
    pub fn parse(s: &str) -> Result<Self, VersionParseError> {
        s.parse()
    }
}

/// A wrapper that associates data with a version for safe migrations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Versioned<T> {
    /// The version when this data was created/last migrated.
    pub version: Version,
    /// The versioned data.
    pub data: T,
}

impl<T> Versioned<T> {
    /// Creates a new versioned wrapper with the current version.
    pub fn new(data: T) -> Self {
        Self {
            version: Version::current(),
            data,
        }
    }

    /// Creates a versioned wrapper with a specific version.
    pub fn with_version(version: Version, data: T) -> Self {
        Self { version, data }
    }

    /// Checks if this versioned data needs migration.
    #[must_use]
    pub fn needs_migration(&self) -> bool {
        let current = Version::current();
        current.is_newer_than(&self.version)
    }

    /// Extracts the inner data, consuming the wrapper.
    pub fn into_inner(self) -> T {
        self.data
    }

    /// Maps the inner data while preserving the version.
    pub fn map<U, F>(self, f: F) -> Versioned<U>
    where
        F: FnOnce(T) -> U,
    {
        Versioned {
            version: self.version,
            data: f(self.data),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        let v1 = Version::new(1, 0, 0);
        let v2 = Version::new(1, 1, 0);
        let v3 = Version::new(2, 0, 0);

        assert!(v2.is_newer_than(&v1));
        assert!(v3.is_newer_than(&v2));
        assert!(!v1.is_newer_than(&v2));
    }

    #[test]
    fn version_compatibility() {
        let v1_0 = Version::new(1, 0, 0);
        let v1_1 = Version::new(1, 1, 0);
        let v2_0 = Version::new(2, 0, 0);

        // Same major, higher minor is compatible
        assert!(v1_0.is_compatible_with(&v1_1));

        // Same major, lower minor is not compatible
        assert!(!v1_1.is_compatible_with(&v1_0));

        // Different major is never compatible
        assert!(!v1_0.is_compatible_with(&v2_0));
    }

    #[test]
    fn versioned_wrapper() {
        let data = Versioned::new("test data");
        assert_eq!(data.version, Version::current());
        assert_eq!(data.into_inner(), "test data");
    }

    #[test]
    fn versioned_map() {
        let data = Versioned::with_version(Version::new(1, 0, 0), 42);
        let mapped = data.map(|n| n.to_string());
        assert_eq!(mapped.version, Version::new(1, 0, 0));
        assert_eq!(mapped.data, "42");
    }

    #[test]
    fn version_parse_valid() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(v, Version::new(1, 2, 3));

        let v: Version = "0.1.0".parse().unwrap();
        assert_eq!(v, Version::new(0, 1, 0));

        let v = Version::parse("  10.20.30  ").unwrap();
        assert_eq!(v, Version::new(10, 20, 30));
    }

    #[test]
    fn version_parse_invalid() {
        assert!(Version::parse("1.2").is_err());
        assert!(Version::parse("1.2.3.4").is_err());
        assert!(Version::parse("abc").is_err());
        assert!(Version::parse("1.two.3").is_err());
        assert!(Version::parse("").is_err());
    }

    #[test]
    fn version_roundtrip_display_parse() {
        let original = Version::new(3, 14, 159);
        let s = original.to_string();
        let parsed = Version::parse(&s).unwrap();
        assert_eq!(original, parsed);
    }
}
