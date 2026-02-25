//! Compilation cache with blake3 hash-based invalidation.
//!
//! Caches compiled WASM artifacts to avoid redundant JS/TS → WASM compilation.
//! Each cache entry is keyed by the blake3 hash of the source content and
//! invalidated when the source, bridge version, or `QuickJS` kernel changes.
//!
//! ## Cache Layout
//!
//! ```text
//! <cache_dir>/
//!   <source-hash>/
//!     plugin.wasm
//!     plugin.toml
//!     cache-meta.json
//! ```
//!
//! ## Atomicity
//!
//! Writes use a temp directory + rename pattern to prevent partial cache entries.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{BridgeError, BridgeResult};

/// Metadata stored alongside each cached compilation artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMeta {
    /// Blake3 hash of the source content (hex-encoded).
    pub source_hash: String,
    /// When the artifact was compiled.
    pub compiled_at: DateTime<Utc>,
    /// Bridge version at compilation time (e.g. `"0.1.0"`).
    pub bridge_version: String,
    /// Blake3 hash of the `QuickJS` kernel used (hex-encoded).
    pub kernel_hash: String,
    /// Blake3 hash of the compiled WASM output (hex-encoded).
    pub wasm_hash: String,
}

const META_FILENAME: &str = "cache-meta.json";
const WASM_FILENAME: &str = "plugin.wasm";
const MANIFEST_FILENAME: &str = "plugin.toml";

/// A successful cache lookup result.
#[derive(Debug)]
pub struct CacheHit {
    /// The compiled WASM bytes.
    pub wasm: Vec<u8>,
    /// The plugin manifest content (`plugin.toml`).
    pub manifest: String,
    /// The cache metadata.
    pub meta: CacheMeta,
}

/// Statistics from a garbage collection run.
#[derive(Debug, Default)]
pub struct GcStats {
    /// Number of cache entries removed.
    pub entries_removed: u64,
    /// Total bytes freed.
    pub bytes_freed: u64,
}

/// Compilation cache for JS/TS → WASM artifacts.
///
/// Stores compiled WASM plugins keyed by the blake3 hash of their source
/// content. Cache entries are invalidated when:
///
/// 1. Source file changes (different blake3 hash → different cache key)
/// 2. Bridge version changes (metadata mismatch → cache miss)
/// 3. `QuickJS` kernel changes (metadata mismatch → cache miss)
pub struct CompilationCache {
    cache_dir: PathBuf,
    kernel_hash: String,
}

impl CompilationCache {
    /// Create a new compilation cache.
    ///
    /// The `cache_dir` is the root directory for all cached artifacts
    /// (e.g. `~/.astrid/cache/plugins/`).
    ///
    /// The `kernel_hash` is the blake3 hex hash of the embedded `QuickJS` kernel,
    /// used to invalidate entries when the kernel changes.
    #[must_use]
    pub fn new(cache_dir: PathBuf, kernel_hash: String) -> Self {
        Self {
            cache_dir,
            kernel_hash,
        }
    }

    /// Check if a compiled artifact exists and is still valid.
    ///
    /// Returns `Some(CacheHit)` if a cached entry exists for the given
    /// `source_hash` and the `bridge_version` and kernel hash both match.
    /// Returns `None` on cache miss or if the entry is stale/corrupt.
    #[must_use]
    pub fn lookup(&self, source_hash: &str, bridge_version: &str) -> Option<CacheHit> {
        // source_hash is a blake3 hex digest — reject anything else to
        // prevent path traversal if a future caller passes untrusted input.
        if !source_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let entry_dir = self.cache_dir.join(source_hash);
        if !entry_dir.is_dir() {
            return None;
        }

        // Read and validate metadata
        let meta_path = entry_dir.join(META_FILENAME);
        let meta_bytes = fs::read(&meta_path).ok()?;
        let meta: CacheMeta = serde_json::from_slice(&meta_bytes).ok()?;

        // Validate invariants: bridge version and kernel hash must match
        if meta.bridge_version != bridge_version || meta.kernel_hash != self.kernel_hash {
            return None;
        }

        // Read cached artifacts
        let wasm = fs::read(entry_dir.join(WASM_FILENAME)).ok()?;
        let manifest = fs::read_to_string(entry_dir.join(MANIFEST_FILENAME)).ok()?;

        // Verify WASM integrity via stored hash
        let actual_wasm_hash = blake3::hash(&wasm).to_hex().to_string();
        if actual_wasm_hash != meta.wasm_hash {
            return None;
        }

        Some(CacheHit {
            wasm,
            manifest,
            meta,
        })
    }

    /// Store a compiled artifact in the cache.
    ///
    /// Writes atomically: all files are written to a temporary directory
    /// first, then renamed into place.
    ///
    /// # Errors
    ///
    /// Returns `BridgeError::Cache` if the cache directory cannot be created
    /// or files cannot be written.
    pub fn store(
        &self,
        source_hash: &str,
        bridge_version: &str,
        wasm: &[u8],
        manifest: &str,
    ) -> BridgeResult<()> {
        // source_hash is a blake3 hex digest — reject anything else to
        // prevent path traversal if a future caller passes untrusted input.
        if !source_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(BridgeError::Cache(format!(
                "invalid source_hash (expected hex): {source_hash}"
            )));
        }

        // Ensure cache root exists
        fs::create_dir_all(&self.cache_dir).map_err(|e| {
            BridgeError::Cache(format!(
                "failed to create cache dir {}: {e}",
                self.cache_dir.display()
            ))
        })?;

        let entry_dir = self.cache_dir.join(source_hash);

        // Write to a temp directory in the same parent (same filesystem for rename)
        let tmp_dir = tempfile::tempdir_in(&self.cache_dir).map_err(|e| {
            BridgeError::Cache(format!("failed to create temp dir for cache write: {e}"))
        })?;

        let wasm_hash = blake3::hash(wasm).to_hex().to_string();

        let meta = CacheMeta {
            source_hash: source_hash.to_string(),
            compiled_at: Utc::now(),
            bridge_version: bridge_version.to_string(),
            kernel_hash: self.kernel_hash.clone(),
            wasm_hash,
        };

        // Write all files to temp dir
        fs::write(tmp_dir.path().join(WASM_FILENAME), wasm)
            .map_err(|e| BridgeError::Cache(format!("failed to write cached WASM: {e}")))?;

        fs::write(tmp_dir.path().join(MANIFEST_FILENAME), manifest)
            .map_err(|e| BridgeError::Cache(format!("failed to write cached manifest: {e}")))?;

        let meta_json = serde_json::to_string_pretty(&meta)
            .map_err(|e| BridgeError::Cache(format!("failed to serialize cache metadata: {e}")))?;
        fs::write(tmp_dir.path().join(META_FILENAME), meta_json)
            .map_err(|e| BridgeError::Cache(format!("failed to write cache metadata: {e}")))?;

        // Atomic swap with backup: move old entry aside, rename temp into place.
        let mut backup_name = entry_dir.file_name().unwrap_or_default().to_os_string();
        backup_name.push(".bak");
        let backup_dir = entry_dir.with_file_name(backup_name);

        if entry_dir.exists() {
            // Clean up any stale backup from a previous failed attempt.
            if backup_dir.exists() {
                let _ = fs::remove_dir_all(&backup_dir);
            }
            fs::rename(&entry_dir, &backup_dir).map_err(|e| {
                BridgeError::Cache(format!(
                    "failed to move existing cache entry to backup: {e}",
                ))
            })?;
        }

        // Persist the temp dir (prevents cleanup on drop) and rename.
        let tmp_path = tmp_dir.keep();
        fs::rename(&tmp_path, &entry_dir).map_err(|e| {
            let _ = fs::remove_dir_all(&tmp_path);
            // Attempt to restore from backup.
            if backup_dir.exists() {
                let _ = fs::rename(&backup_dir, &entry_dir);
            }
            BridgeError::Cache(format!(
                "failed to rename temp dir to {}: {e}",
                entry_dir.display()
            ))
        })?;

        // Success — remove backup.
        if backup_dir.exists() {
            let _ = fs::remove_dir_all(&backup_dir);
        }

        Ok(())
    }

    /// Evict stale entries older than `max_age_days` or exceeding `max_size_bytes`.
    ///
    /// Entries are first pruned by age, then by total size (oldest first).
    ///
    /// # Errors
    ///
    /// Returns `BridgeError::Cache` if the cache directory cannot be read.
    pub fn gc(&self, max_age_days: u64, max_size_bytes: u64) -> BridgeResult<GcStats> {
        let mut stats = GcStats::default();

        if !self.cache_dir.exists() {
            return Ok(stats);
        }

        let now = Utc::now();
        let max_age = chrono::Duration::days(i64::try_from(max_age_days).unwrap_or(i64::MAX));

        // Collect all cache entries with their metadata and sizes
        let mut entries: Vec<(PathBuf, CacheMeta, u64)> = Vec::new();

        let read_dir = fs::read_dir(&self.cache_dir).map_err(|e| {
            BridgeError::Cache(format!(
                "failed to read cache dir {}: {e}",
                self.cache_dir.display()
            ))
        })?;

        for entry in read_dir {
            let Ok(entry) = entry else { continue };

            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let meta_path = path.join(META_FILENAME);
            let Some(meta) = fs::read(&meta_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<CacheMeta>(&bytes).ok())
            else {
                // Corrupt entry — evict it
                let size = dir_size(&path);
                let _ = fs::remove_dir_all(&path);
                stats.entries_removed = stats.entries_removed.saturating_add(1);
                stats.bytes_freed = stats.bytes_freed.saturating_add(size);
                continue;
            };

            let size = dir_size(&path);
            entries.push((path, meta, size));
        }

        // Phase 1: Evict entries older than max_age
        entries.retain(|(path, meta, size)| {
            if now.signed_duration_since(meta.compiled_at) > max_age {
                let _ = fs::remove_dir_all(path);
                stats.entries_removed = stats.entries_removed.saturating_add(1);
                stats.bytes_freed = stats.bytes_freed.saturating_add(*size);
                false
            } else {
                true
            }
        });

        // Phase 2: Evict oldest entries until total size is under max_size_bytes
        let total_size: u64 = entries.iter().map(|(_, _, s)| s).sum();
        if total_size > max_size_bytes {
            // Sort oldest first
            entries.sort_by_key(|(_, meta, _)| meta.compiled_at);

            let mut current_size = total_size;
            for (path, _, size) in &entries {
                if current_size <= max_size_bytes {
                    break;
                }
                let _ = fs::remove_dir_all(path);
                stats.entries_removed = stats.entries_removed.saturating_add(1);
                stats.bytes_freed = stats.bytes_freed.saturating_add(*size);
                current_size = current_size.saturating_sub(*size);
            }
        }

        Ok(stats)
    }
}

/// Compute the total size of a directory and all its contents.
fn dir_size(path: &Path) -> u64 {
    let mut total: u64 = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_file() {
                    total = total.saturating_add(metadata.len());
                } else if metadata.is_dir() {
                    total = total.saturating_add(dir_size(&entry.path()));
                }
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache() -> (tempfile::TempDir, CompilationCache) {
        let dir = tempfile::tempdir().unwrap();
        let cache =
            CompilationCache::new(dir.path().join("plugins"), "kernel_hash_abc".to_string());
        (dir, cache)
    }

    #[test]
    fn lookup_miss_on_empty_cache() {
        let (_dir, cache) = temp_cache();
        assert!(cache.lookup("dead0000beef", "0.1.0").is_none());
    }

    #[test]
    fn store_and_lookup_hit() {
        let (_dir, cache) = temp_cache();
        let wasm = b"\0asm\x01\x00\x00\x00fake wasm content";
        let manifest = "id = \"test-plugin\"\nversion = \"1.0.0\"";

        cache
            .store("aa11bb22cc33", "0.1.0", wasm, manifest)
            .unwrap();

        let hit = cache.lookup("aa11bb22cc33", "0.1.0").unwrap();
        assert_eq!(hit.wasm, wasm);
        assert_eq!(hit.manifest, manifest);
        assert_eq!(hit.meta.source_hash, "aa11bb22cc33");
        assert_eq!(hit.meta.bridge_version, "0.1.0");
        assert_eq!(hit.meta.kernel_hash, "kernel_hash_abc");
    }

    #[test]
    fn lookup_miss_on_bridge_version_change() {
        let (_dir, cache) = temp_cache();
        let wasm = b"wasm bytes";
        let manifest = "manifest";

        cache.store("dd44ee55", "0.1.0", wasm, manifest).unwrap();

        // Same source hash but different bridge version → miss
        assert!(cache.lookup("dd44ee55", "0.2.0").is_none());
    }

    #[test]
    fn lookup_miss_on_kernel_hash_change() {
        let (dir, cache) = temp_cache();
        let wasm = b"wasm bytes";
        let manifest = "manifest";

        cache.store("dd44ee55", "0.1.0", wasm, manifest).unwrap();

        // Create a new cache with a different kernel hash pointing at same dir
        let cache2 = CompilationCache::new(
            dir.path().join("plugins"),
            "different_kernel_hash".to_string(),
        );
        assert!(cache2.lookup("dd44ee55", "0.1.0").is_none());
    }

    #[test]
    fn store_overwrites_existing_entry() {
        let (_dir, cache) = temp_cache();

        cache
            .store("dd44ee55", "0.1.0", b"old wasm", "old manifest")
            .unwrap();
        cache
            .store("dd44ee55", "0.1.0", b"new wasm", "new manifest")
            .unwrap();

        let hit = cache.lookup("dd44ee55", "0.1.0").unwrap();
        assert_eq!(hit.wasm, b"new wasm");
        assert_eq!(hit.manifest, "new manifest");
    }

    #[test]
    fn lookup_miss_on_corrupt_metadata() {
        let (_dir, cache) = temp_cache();

        // Manually create a corrupt entry
        let entry_dir = cache.cache_dir.join("cccc0000");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join(META_FILENAME), "not valid json").unwrap();
        fs::write(entry_dir.join(WASM_FILENAME), b"wasm").unwrap();
        fs::write(entry_dir.join(MANIFEST_FILENAME), "manifest").unwrap();

        assert!(cache.lookup("cccc0000", "0.1.0").is_none());
    }

    #[test]
    fn lookup_miss_on_wasm_integrity_failure() {
        let (_dir, cache) = temp_cache();

        // Store a valid entry
        cache
            .store("dd44ee55", "0.1.0", b"original wasm", "manifest")
            .unwrap();

        // Tamper with the WASM file
        let wasm_path = cache.cache_dir.join("dd44ee55").join(WASM_FILENAME);
        fs::write(wasm_path, b"tampered wasm").unwrap();

        // Lookup should fail integrity check
        assert!(cache.lookup("dd44ee55", "0.1.0").is_none());
    }

    #[test]
    fn gc_removes_old_entries() {
        let (_dir, cache) = temp_cache();

        // Store an entry, then backdate its metadata
        cache
            .store("0000aaaa", "0.1.0", b"wasm", "manifest")
            .unwrap();

        let meta_path = cache.cache_dir.join("0000aaaa").join(META_FILENAME);
        let mut meta: CacheMeta = serde_json::from_slice(&fs::read(&meta_path).unwrap()).unwrap();
        meta.compiled_at = Utc::now() - chrono::Duration::days(60);
        fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap()).unwrap();

        let stats = cache.gc(30, u64::MAX).unwrap();
        assert_eq!(stats.entries_removed, 1);
        assert!(stats.bytes_freed > 0);
        assert!(!cache.cache_dir.join("0000aaaa").exists());
    }

    #[test]
    fn gc_removes_entries_over_size_limit() {
        let (_dir, cache) = temp_cache();

        // Store several entries
        let big_wasm = vec![0u8; 1024];
        cache
            .store("aaaa1111", "0.1.0", &big_wasm, "manifest a")
            .unwrap();

        // Backdate "aaaa1111" so it's the oldest
        let meta_path = cache.cache_dir.join("aaaa1111").join(META_FILENAME);
        let mut meta: CacheMeta = serde_json::from_slice(&fs::read(&meta_path).unwrap()).unwrap();
        meta.compiled_at = Utc::now() - chrono::Duration::days(5);
        fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap()).unwrap();

        cache
            .store("bbbb2222", "0.1.0", &big_wasm, "manifest b")
            .unwrap();

        // GC with a tiny size limit — should evict oldest entries
        let stats = cache.gc(365, 100).unwrap();
        assert!(stats.entries_removed >= 1);
        assert!(stats.bytes_freed > 0);
    }

    #[test]
    fn gc_evicts_corrupt_entries() {
        let (_dir, cache) = temp_cache();

        // Create a corrupt entry (no valid metadata)
        let entry_dir = cache.cache_dir.join("cccc0000");
        fs::create_dir_all(&entry_dir).unwrap();
        fs::write(entry_dir.join("junk.dat"), b"some bytes").unwrap();

        let stats = cache.gc(30, u64::MAX).unwrap();
        assert_eq!(stats.entries_removed, 1);
        assert!(!entry_dir.exists());
    }

    #[test]
    fn gc_on_empty_cache() {
        let (_dir, cache) = temp_cache();
        let stats = cache.gc(30, 500_000_000).unwrap();
        assert_eq!(stats.entries_removed, 0);
        assert_eq!(stats.bytes_freed, 0);
    }

    #[test]
    fn gc_on_nonexistent_dir() {
        let cache = CompilationCache::new(
            PathBuf::from("/tmp/claude/nonexistent-cache-dir-test"),
            "hash".to_string(),
        );
        let stats = cache.gc(30, 500_000_000).unwrap();
        assert_eq!(stats.entries_removed, 0);
        assert_eq!(stats.bytes_freed, 0);
    }

    #[test]
    fn multiple_entries_coexist() {
        let (_dir, cache) = temp_cache();

        cache
            .store("ff0011aa", "0.1.0", b"wasm_a", "manifest_a")
            .unwrap();
        cache
            .store("ff0022bb", "0.1.0", b"wasm_b", "manifest_b")
            .unwrap();

        let hit_a = cache.lookup("ff0011aa", "0.1.0").unwrap();
        let hit_b = cache.lookup("ff0022bb", "0.1.0").unwrap();

        assert_eq!(hit_a.wasm, b"wasm_a");
        assert_eq!(hit_b.wasm, b"wasm_b");
    }

    #[test]
    fn meta_serialization_roundtrip() {
        let meta = CacheMeta {
            source_hash: "abc123".to_string(),
            compiled_at: Utc::now(),
            bridge_version: "0.1.0".to_string(),
            kernel_hash: "def456".to_string(),
            wasm_hash: "789aaa".to_string(),
        };

        let json = serde_json::to_string_pretty(&meta).unwrap();
        let decoded: CacheMeta = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.source_hash, meta.source_hash);
        assert_eq!(decoded.bridge_version, meta.bridge_version);
        assert_eq!(decoded.kernel_hash, meta.kernel_hash);
        assert_eq!(decoded.wasm_hash, meta.wasm_hash);
    }
}
