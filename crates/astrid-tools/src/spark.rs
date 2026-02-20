//! Spark identity configuration — the agent's living self-description.
//!
//! The spark is a compact identity file (`spark.toml`) that defines the agent's
//! name, role, personality, communication style, and core philosophy. It can be
//! seeded by the user via `[spark]` in `config.toml` and evolved by the agent
//! itself via the `spark` built-in tool.
//!
//! Read priority: `spark.toml` > `[spark]` in config > empty (default behavior).
//! Write target: `spark.toml` only (agent never touches `config.toml`).

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ToolError;

/// Maximum length for short fields (callsign, class, aura, signal).
const MAX_SHORT_FIELD_LEN: usize = 100;

/// Maximum length for the core field.
const MAX_CORE_LEN: usize = 2000;

/// Maximum spark file size (64 KB).
const MAX_SPARK_FILE_SIZE: u64 = 64 * 1024;

/// Agent identity configuration.
///
/// All fields default to empty strings. When all fields are empty, the agent
/// uses the default "Astrid" identity (zero behavior change).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SparkConfig {
    /// Agent's name (e.g. "Stellar", "Nova", "Orion").
    pub callsign: String,
    /// Role archetype (e.g. "navigator", "engineer", "sentinel").
    pub class: String,
    /// Personality energy (e.g. "calm", "sharp", "warm", "analytical").
    pub aura: String,
    /// Communication style (e.g. "formal", "concise", "casual", "poetic").
    pub signal: String,
    /// Soul/philosophy — free-form values, learned patterns, personality depth.
    pub core: String,
}

impl SparkConfig {
    /// Returns `true` when all fields are empty (no identity configured).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.callsign.is_empty()
            && self.class.is_empty()
            && self.aura.is_empty()
            && self.signal.is_empty()
            && self.core.is_empty()
    }

    /// Sanitize all fields: trim whitespace, enforce length limits, and
    /// remove newlines from short fields. Whitespace-only fields become empty.
    pub fn sanitize(&mut self) {
        sanitize_short_field(&mut self.callsign);
        sanitize_short_field(&mut self.class);
        sanitize_short_field(&mut self.aura);
        sanitize_short_field(&mut self.signal);
        // Core allows newlines but is length-limited
        let trimmed = self.core.trim().to_string();
        if trimmed.len() > MAX_CORE_LEN {
            // Truncate at a char boundary
            self.core = truncate_at_char_boundary(&trimmed, MAX_CORE_LEN);
        } else {
            self.core = trimmed;
        }
    }

    /// Load a spark config from a TOML file.
    ///
    /// Returns `None` if the file is missing, too large, or cannot be parsed.
    #[must_use]
    pub fn load_from_file(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        if metadata.len() > MAX_SPARK_FILE_SIZE {
            return None;
        }
        let contents = std::fs::read_to_string(path).ok()?;
        let mut config: Self = toml::from_str(&contents).ok()?;
        config.sanitize();
        Some(config)
    }

    /// Save the spark config to a TOML file.
    ///
    /// Creates parent directories if they don't exist. Sanitizes before writing.
    ///
    /// # Errors
    ///
    /// Returns a [`ToolError`] if directory creation or file writing fails.
    pub fn save_to_file(&self, path: &Path) -> Result<(), ToolError> {
        let mut sanitized = self.clone();
        sanitized.sanitize();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let toml_str =
            toml::to_string_pretty(&sanitized).map_err(|e| ToolError::Other(e.to_string()))?;
        // Atomic write: write to a temp file in the same directory, then rename.
        // This prevents partial writes from corrupting the file on process kill.
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(toml_str.as_bytes())?;
        tmp.persist(path)
            .map_err(|e| ToolError::Other(format!("failed to persist spark file: {e}")))?;
        Ok(())
    }

    /// Build the identity preamble for system prompt injection.
    ///
    /// When all fields are empty, returns `None` (caller should use the default
    /// "Astrid" opening). When at least one field is set, builds a
    /// structured identity block.
    #[must_use]
    pub fn build_preamble(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }

        let mut sections = Vec::new();

        // Opening line: "You are {callsign}" or "You are an AI agent"
        let opening = if self.callsign.is_empty() {
            if self.class.is_empty() {
                "You are an AI agent.".to_string()
            } else {
                format!("You are an AI agent, a {}.", self.class)
            }
        } else if self.class.is_empty() {
            format!("You are {}.", self.callsign)
        } else {
            format!("You are {}, a {}.", self.callsign, self.class)
        };
        sections.push(opening);

        if !self.aura.is_empty() {
            sections.push(format!("\n# Personality\n{}", self.aura));
        }

        if !self.signal.is_empty() {
            sections.push(format!("\n# Communication Style\n{}", self.signal));
        }

        if !self.core.is_empty() {
            sections.push(format!("\n# Core Directives\n{}", self.core));
        }

        Some(sections.join("\n"))
    }

    /// Merge another spark into this one, only updating non-empty fields.
    pub fn merge(&mut self, other: &SparkConfig) {
        if !other.callsign.is_empty() {
            self.callsign.clone_from(&other.callsign);
        }
        if !other.class.is_empty() {
            self.class.clone_from(&other.class);
        }
        if !other.aura.is_empty() {
            self.aura.clone_from(&other.aura);
        }
        if !other.signal.is_empty() {
            self.signal.clone_from(&other.signal);
        }
        if !other.core.is_empty() {
            self.core.clone_from(&other.core);
        }
    }

    /// Merge optional field updates. `None` = don't touch, `Some(value)` = set
    /// (including `Some("")` to clear a field).
    pub fn merge_optional(
        &mut self,
        callsign: Option<&str>,
        class: Option<&str>,
        aura: Option<&str>,
        signal: Option<&str>,
        core: Option<&str>,
    ) {
        if let Some(v) = callsign {
            self.callsign = v.to_string();
        }
        if let Some(v) = class {
            self.class = v.to_string();
        }
        if let Some(v) = aura {
            self.aura = v.to_string();
        }
        if let Some(v) = signal {
            self.signal = v.to_string();
        }
        if let Some(v) = core {
            self.core = v.to_string();
        }
    }
}

/// Sanitize a short spark field: trim, collapse to empty if whitespace-only,
/// remove newlines, and enforce length limit.
fn sanitize_short_field(field: &mut String) {
    let trimmed = field.trim().replace(['\n', '\r'], " ");
    if trimmed.len() > MAX_SHORT_FIELD_LEN {
        *field = truncate_at_char_boundary(&trimmed, MAX_SHORT_FIELD_LEN);
    } else {
        *field = trimmed;
    }
}

/// RAII guard that holds a file lock and cleans up the lock file on drop.
pub(crate) struct SparkLockGuard {
    file: std::fs::File,
    path: std::path::PathBuf,
}

impl Drop for SparkLockGuard {
    fn drop(&mut self) {
        // Explicitly release the advisory lock before removing the file.
        // fs2::FileExt::unlock requires the trait in scope.
        let _ = <std::fs::File as fs2::FileExt>::unlock(&self.file);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Acquire an exclusive lock on a `.spark.lock` file next to the spark file.
///
/// Returns a [`SparkLockGuard`] that holds the lock. When dropped, the lock
/// is released and the lock file is cleaned up.
pub(crate) fn acquire_spark_lock(spark_path: &Path) -> Result<SparkLockGuard, ToolError> {
    use fs2::FileExt;

    let lock_path = spark_path.with_extension("lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;
    file.lock_exclusive()
        .map_err(|e| ToolError::Other(format!("failed to acquire spark lock: {e}")))?;
    Ok(SparkLockGuard {
        file,
        path: lock_path,
    })
}

use super::truncate::truncate_at_char_boundary;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_empty() {
        let spark = SparkConfig::default();
        assert!(spark.is_empty());
    }

    #[test]
    fn test_not_empty_with_callsign() {
        let spark = SparkConfig {
            callsign: "Stellar".to_string(),
            ..Default::default()
        };
        assert!(!spark.is_empty());
    }

    #[test]
    fn test_load_from_missing_file() {
        assert!(SparkConfig::load_from_file(Path::new("/nonexistent/spark.toml")).is_none());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spark.toml");

        let spark = SparkConfig {
            callsign: "Nova".to_string(),
            class: "engineer".to_string(),
            aura: "sharp".to_string(),
            signal: "concise".to_string(),
            core: "I value precision.".to_string(),
        };

        spark.save_to_file(&path).unwrap();
        let loaded = SparkConfig::load_from_file(&path).unwrap();

        assert_eq!(loaded.callsign, "Nova");
        assert_eq!(loaded.class, "engineer");
        assert_eq!(loaded.aura, "sharp");
        assert_eq!(loaded.signal, "concise");
        assert_eq!(loaded.core, "I value precision.");
    }

    #[test]
    fn test_build_preamble_empty_returns_none() {
        let spark = SparkConfig::default();
        assert!(spark.build_preamble().is_none());
    }

    #[test]
    fn test_build_preamble_full() {
        let spark = SparkConfig {
            callsign: "Stellar".to_string(),
            class: "navigator".to_string(),
            aura: "calm".to_string(),
            signal: "formal".to_string(),
            core: "I value clarity.".to_string(),
        };

        let preamble = spark.build_preamble().unwrap();
        assert!(preamble.contains("You are Stellar, a navigator."));
        assert!(preamble.contains("# Personality\ncalm"));
        assert!(preamble.contains("# Communication Style\nformal"));
        assert!(preamble.contains("# Core Directives\nI value clarity."));
    }

    #[test]
    fn test_build_preamble_callsign_only() {
        let spark = SparkConfig {
            callsign: "Orion".to_string(),
            ..Default::default()
        };

        let preamble = spark.build_preamble().unwrap();
        assert!(preamble.contains("You are Orion."));
        assert!(!preamble.contains("# Personality"));
        assert!(!preamble.contains("# Communication Style"));
        assert!(!preamble.contains("# Core Directives"));
    }

    #[test]
    fn test_build_preamble_class_only() {
        let spark = SparkConfig {
            class: "sentinel".to_string(),
            ..Default::default()
        };

        let preamble = spark.build_preamble().unwrap();
        assert!(preamble.contains("You are an AI agent, a sentinel."));
    }

    #[test]
    fn test_merge_updates_non_empty_fields() {
        let mut base = SparkConfig {
            callsign: "Nova".to_string(),
            class: "engineer".to_string(),
            aura: "sharp".to_string(),
            signal: String::new(),
            core: "Original core.".to_string(),
        };

        let update = SparkConfig {
            callsign: String::new(), // should NOT overwrite
            class: "navigator".to_string(),
            aura: String::new(), // should NOT overwrite
            signal: "concise".to_string(),
            core: "Evolved core.".to_string(),
        };

        base.merge(&update);

        assert_eq!(base.callsign, "Nova"); // preserved
        assert_eq!(base.class, "navigator"); // updated
        assert_eq!(base.aura, "sharp"); // preserved
        assert_eq!(base.signal, "concise"); // updated
        assert_eq!(base.core, "Evolved core."); // updated
    }

    #[test]
    fn test_load_partial_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spark.toml");
        std::fs::write(&path, "callsign = \"Astrid\"\n").unwrap();

        let spark = SparkConfig::load_from_file(&path).unwrap();
        assert_eq!(spark.callsign, "Astrid");
        assert!(spark.class.is_empty());
        assert!(spark.aura.is_empty());
    }

    #[test]
    fn test_merge_optional_clears_fields() {
        let mut spark = SparkConfig {
            callsign: "Nova".to_string(),
            class: "engineer".to_string(),
            aura: "sharp".to_string(),
            signal: "formal".to_string(),
            core: "I value clarity.".to_string(),
        };

        // None = don't touch, Some("") = clear, Some(value) = update
        spark.merge_optional(None, Some(""), None, Some("concise"), None);

        assert_eq!(spark.callsign, "Nova"); // untouched
        assert!(spark.class.is_empty()); // cleared
        assert_eq!(spark.aura, "sharp"); // untouched
        assert_eq!(spark.signal, "concise"); // updated
        assert_eq!(spark.core, "I value clarity."); // untouched
    }

    #[test]
    fn test_sanitize_trims_whitespace() {
        let mut spark = SparkConfig {
            callsign: "  Stellar  ".to_string(),
            class: "   ".to_string(), // whitespace-only becomes empty
            ..Default::default()
        };
        spark.sanitize();
        assert_eq!(spark.callsign, "Stellar");
        assert!(spark.class.is_empty());
    }

    #[test]
    fn test_sanitize_removes_newlines_from_short_fields() {
        let mut spark = SparkConfig {
            callsign: "Stellar\nEvil".to_string(),
            class: "nav\rigator".to_string(),
            ..Default::default()
        };
        spark.sanitize();
        assert_eq!(spark.callsign, "Stellar Evil");
        assert_eq!(spark.class, "nav igator");
    }

    #[test]
    fn test_sanitize_truncates_long_fields() {
        let mut spark = SparkConfig {
            callsign: "x".repeat(200),
            core: "y".repeat(3000),
            ..Default::default()
        };
        spark.sanitize();
        assert!(spark.callsign.len() <= MAX_SHORT_FIELD_LEN);
        assert!(spark.core.len() <= MAX_CORE_LEN);
    }

    #[test]
    fn test_sanitize_handles_multibyte_truncation() {
        // 100 emoji = 400 bytes, should truncate at a char boundary
        let mut spark = SparkConfig {
            callsign: "🔥".repeat(100),
            ..Default::default()
        };
        spark.sanitize();
        assert!(spark.callsign.len() <= MAX_SHORT_FIELD_LEN);
        // Ensure we didn't split a multi-byte char
        assert!(spark.callsign.is_char_boundary(spark.callsign.len()));
    }

    #[test]
    fn test_load_rejects_oversized_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spark.toml");
        // Write a file larger than MAX_SPARK_FILE_SIZE
        let content = "x".repeat(65 * 1024 + 1);
        std::fs::write(&path, content).unwrap();
        assert!(SparkConfig::load_from_file(&path).is_none());
    }
}
