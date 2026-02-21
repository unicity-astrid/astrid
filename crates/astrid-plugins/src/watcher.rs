//! Hot-reload file watcher for plugins.
//!
//! Watches plugin source directories for file changes, debounces events,
//! and emits [`WatchEvent`]s when plugin source content actually changes
//! (verified via blake3 hashing). Runs as a daemon background task,
//! enabled by default via `gateway.watch_plugins` in config.
//!
//! # Architecture
//!
//! ```text
//! filesystem events (notify)
//!   → filter ignored dirs (node_modules, target, dist, .git)
//!   → map to plugin directory
//!   → debounce 500ms per plugin
//!   → blake3 hash source tree
//!   → compare to cached hash
//!   → emit WatchEvent::PluginChanged
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::discovery::MANIFEST_FILE_NAME;
use crate::error::PluginResult;

/// Default debounce interval for file change events.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(500);

/// Directory names to ignore during file watching.
pub const IGNORED_DIRS: &[&str] = &["node_modules", "target", "dist", ".git"];

/// File extensions to exclude from source hashing (generated artifacts).
const IGNORED_EXTENSIONS: &[&str] = &["wasm"];

/// Specific filenames to exclude from source hashing (generated files).
const IGNORED_FILENAMES: &[&str] = &["astrid_bridge.mjs"];

/// Events emitted by the plugin watcher.
#[derive(Debug, Clone)]
pub enum WatchEvent {
    /// A plugin's source files changed and may need recompilation.
    PluginChanged {
        /// The plugin's root directory (contains `plugin.toml` or `openclaw.plugin.json`).
        plugin_dir: PathBuf,
        /// blake3 hash of the plugin's source tree after the change.
        source_hash: String,
    },
    /// Watcher encountered a non-fatal error.
    Error(String),
}

/// Configuration for the plugin watcher.
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    /// Root directories to watch. Each should contain plugin subdirectories.
    pub watch_paths: Vec<PathBuf>,
    /// Debounce interval. File changes within this window are coalesced.
    pub debounce: Duration,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            watch_paths: Vec::new(),
            debounce: DEFAULT_DEBOUNCE,
        }
    }
}

/// Watches plugin source directories for changes and emits [`WatchEvent`]s.
///
/// Uses the `notify` crate for cross-platform filesystem watching and blake3
/// hashing to prevent unnecessary recompilation when file contents haven't
/// actually changed.
pub struct PluginWatcher {
    config: WatcherConfig,
    /// blake3 hash cache per plugin directory.
    hash_cache: HashMap<PathBuf, String>,
    /// The `notify` filesystem watcher handle. Kept alive for the duration
    /// of the watcher's lifetime — dropping it stops filesystem monitoring.
    watcher: RecommendedWatcher,
    /// Receives raw filesystem events from the `notify` callback thread.
    raw_rx: mpsc::UnboundedReceiver<notify::Result<Event>>,
    /// Sends processed [`WatchEvent`]s to the consumer.
    event_tx: mpsc::Sender<WatchEvent>,
}

impl PluginWatcher {
    /// Create a new plugin watcher.
    ///
    /// Returns the watcher and a receiver for [`WatchEvent`]s. Call
    /// [`run()`](Self::run) to start the event loop.
    ///
    /// # Errors
    ///
    /// Returns an error if the filesystem watcher cannot be initialized.
    pub fn new(config: WatcherConfig) -> PluginResult<(Self, mpsc::Receiver<WatchEvent>)> {
        let (raw_tx, raw_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::channel(64);

        let watcher = RecommendedWatcher::new(
            move |res| {
                let _ = raw_tx.send(res);
            },
            notify::Config::default(),
        )
        .map_err(|e| {
            crate::error::PluginError::Io(std::io::Error::other(format!("filesystem watcher: {e}")))
        })?;

        Ok((
            Self {
                config,
                hash_cache: HashMap::new(),
                watcher,
                raw_rx,
                event_tx,
            },
            event_rx,
        ))
    }

    /// Run the watcher event loop.
    ///
    /// Starts watching all configured paths and processes filesystem events
    /// until the raw event channel closes (i.e., the `notify` watcher is dropped
    /// or encounters a fatal error).
    pub async fn run(mut self) {
        // Start watching configured paths.
        for path in &self.config.watch_paths {
            if path.exists() {
                match self.watcher.watch(path, RecursiveMode::Recursive) {
                    Ok(()) => info!(path = %path.display(), "Watching plugin directory"),
                    Err(e) => warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to watch directory"
                    ),
                }
            } else {
                warn!(path = %path.display(), "Watch path does not exist, skipping");
            }
        }

        let debounce = self.config.debounce;
        let mut pending: HashMap<PathBuf, tokio::time::Instant> = HashMap::new();

        loop {
            let next_deadline = pending.values().copied().min();

            tokio::select! {
                biased;

                // Fire debounced events (check timeouts first).
                () = async {
                    match next_deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    let now = tokio::time::Instant::now();
                    let ready: Vec<PathBuf> = pending
                        .iter()
                        .filter(|(_, deadline)| **deadline <= now)
                        .map(|(path, _)| path.clone())
                        .collect();

                    for plugin_dir in ready {
                        pending.remove(&plugin_dir);
                        if !self.process_plugin_change(&plugin_dir).await {
                            return; // Receiver dropped, stop the watcher.
                        }
                    }
                }

                // Process incoming FS events.
                event = self.raw_rx.recv() => {
                    match event {
                        Some(Ok(ev)) => {
                            self.handle_raw_event(&ev, &mut pending, debounce);
                        }
                        Some(Err(e)) => {
                            warn!(error = %e, "Filesystem watcher error");
                            if self.event_tx.send(WatchEvent::Error(e.to_string())).await.is_err() {
                                debug!("Event receiver dropped, stopping watcher");
                                return;
                            }
                        }
                        None => {
                            debug!("Filesystem watcher channel closed, stopping");
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Map a raw `notify` event to a plugin directory and reset its debounce timer.
    fn handle_raw_event(
        &self,
        event: &Event,
        pending: &mut HashMap<PathBuf, tokio::time::Instant>,
        debounce: Duration,
    ) {
        // Only process content-changing events.
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {},
            _ => return,
        }

        for path in &event.paths {
            if is_in_ignored_dir(path) {
                continue;
            }

            if let Some(plugin_dir) = self.resolve_plugin_dir(path) {
                debug!(
                    path = %path.display(),
                    plugin_dir = %plugin_dir.display(),
                    kind = ?event.kind,
                    "File change detected in plugin"
                );
                #[allow(clippy::arithmetic_side_effects)]
                // Instant + Duration cannot overflow in practice
                let deadline = tokio::time::Instant::now() + debounce;
                pending.insert(plugin_dir, deadline);
            }
        }
    }

    /// Walk up from a changed file to find its parent plugin directory.
    ///
    /// A plugin directory is identified by containing `plugin.toml` or
    /// `openclaw.plugin.json`. Starts from the parent of the changed path
    /// to avoid an unnecessary `stat` syscall on the path itself.
    fn resolve_plugin_dir(&self, path: &Path) -> Option<PathBuf> {
        // Start from the parent directory — for files in the plugin root
        // (including manifests like plugin.toml), parent() gives the plugin
        // directory directly.
        let mut current = path.parent()?.to_path_buf();

        loop {
            if current.join(MANIFEST_FILE_NAME).exists()
                || current.join("openclaw.plugin.json").exists()
            {
                return Some(current);
            }

            // Stop at watch roots to avoid traversing the entire filesystem.
            // Compare via components() to handle trailing slashes and
            // redundant separators from notify event paths.
            if self
                .config
                .watch_paths
                .iter()
                .any(|root| current.components().eq(root.components()))
            {
                return None;
            }

            current = current.parent()?.to_path_buf();
        }
    }

    /// Hash the plugin's source tree and emit an event if the hash changed.
    ///
    /// Returns `false` if the event receiver has been dropped (caller should
    /// stop the watcher loop to avoid wasting resources).
    async fn process_plugin_change(&mut self, plugin_dir: &Path) -> bool {
        // Run the recursive file hashing on a blocking thread to avoid
        // starving the Tokio worker (plugin directories can be large).
        let dir = plugin_dir.to_path_buf();
        let hash_result = match tokio::task::spawn_blocking(move || compute_source_hash(&dir)).await
        {
            Ok(result) => result,
            Err(e) => {
                warn!(error = %e, "Hash task was cancelled");
                return true;
            },
        };

        match hash_result {
            Ok(new_hash) => {
                if self
                    .hash_cache
                    .get(plugin_dir)
                    .is_some_and(|h| h == &new_hash)
                {
                    debug!(
                        plugin_dir = %plugin_dir.display(),
                        "Source hash unchanged, skipping recompilation"
                    );
                    return true;
                }

                info!(
                    plugin_dir = %plugin_dir.display(),
                    hash = %new_hash,
                    "Plugin source changed, triggering reload"
                );
                self.hash_cache
                    .insert(plugin_dir.to_path_buf(), new_hash.clone());

                if self
                    .event_tx
                    .send(WatchEvent::PluginChanged {
                        plugin_dir: plugin_dir.to_path_buf(),
                        source_hash: new_hash,
                    })
                    .await
                    .is_err()
                {
                    debug!("Event receiver dropped, stopping watcher");
                    return false;
                }
            },
            Err(e) => {
                warn!(
                    plugin_dir = %plugin_dir.display(),
                    error = %e,
                    "Failed to hash plugin source tree"
                );
                if self
                    .event_tx
                    .send(WatchEvent::Error(format!(
                        "Hash failed for {}: {e}",
                        plugin_dir.display()
                    )))
                    .await
                    .is_err()
                {
                    debug!("Event receiver dropped, stopping watcher");
                    return false;
                }
            },
        }
        true
    }
}

/// Check if a path contains any ignored directory component.
fn is_in_ignored_dir(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| IGNORED_DIRS.contains(&s))
    })
}

/// Compute a deterministic blake3 hash over all source files in a directory.
///
/// Files are sorted by relative path for deterministic output. The hash covers
/// both file paths (to detect renames) and file contents.
///
/// Directories in [`IGNORED_DIRS`] are skipped. Generated artifacts (`.wasm`
/// files, `astrid_bridge.mjs`) are excluded to prevent recompilation feedback
/// loops.
///
/// # Errors
///
/// Returns an error if the directory itself cannot be read. Individual
/// unreadable files (e.g. deleted between enumeration and read) are
/// skipped with a debug log rather than failing the entire hash.
pub fn compute_source_hash(dir: &Path) -> std::io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut paths = Vec::new();
    collect_source_paths(dir, &mut paths)?;
    paths.sort();

    for path in &paths {
        // Only feed path + content into the hasher when both are available.
        // This preserves domain-separation: each file contributes
        // [path_len][path_bytes][content_len][content_bytes] atomically.
        // Skipping unreadable files entirely (rather than feeding a partial
        // record) avoids ambiguous hash states on transient TOCTOU races.
        let Ok(rel) = path.strip_prefix(dir) else {
            continue;
        };
        match std::fs::read(path) {
            Ok(content) => {
                let rel_bytes = rel.to_string_lossy();
                let rel_bytes = rel_bytes.as_bytes();
                hasher.update(&(rel_bytes.len() as u64).to_le_bytes());
                hasher.update(rel_bytes);
                hasher.update(&(content.len() as u64).to_le_bytes());
                hasher.update(&content);
            },
            Err(e) => {
                debug!(path = %path.display(), error = %e, "Skipping unreadable file in hash");
            },
        }
    }

    Ok(hasher.finalize().to_hex().to_string())
}

/// Recursively collect source file paths, skipping [`IGNORED_DIRS`] and
/// generated artifacts.
///
/// Uses `DirEntry::file_type()` (lstat-based) to avoid following symlinks,
/// which prevents infinite recursion on symlink loops.
fn collect_source_paths(dir: &Path, paths: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)? {
        // Skip individual entries that fail (e.g. deleted between readdir
        // iteration and stat — common during rapid build cycles).
        let Ok(entry) = entry else { continue };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();

        // Skip symlinks entirely — prevents infinite loops and directory
        // escapes in user-controlled plugin directories.
        if file_type.is_symlink() {
            continue;
        }

        if file_type.is_dir() {
            let name = entry.file_name();
            if IGNORED_DIRS
                .iter()
                .any(|&d| d == name.to_string_lossy().as_ref())
            {
                continue;
            }
            collect_source_paths(&path, paths)?;
        } else if file_type.is_file() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Skip generated artifacts.
            if IGNORED_FILENAMES.contains(&name_str.as_ref()) {
                continue;
            }
            if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| IGNORED_EXTENSIONS.contains(&ext))
            {
                continue;
            }

            paths.push(path);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_compute_source_hash_deterministic() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("b.txt"), "world").unwrap();

        let hash1 = compute_source_hash(dir.path()).unwrap();
        let hash2 = compute_source_hash(dir.path()).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_compute_source_hash_changes_on_content_change() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();

        let hash1 = compute_source_hash(dir.path()).unwrap();

        std::fs::write(dir.path().join("a.txt"), "world").unwrap();

        let hash2 = compute_source_hash(dir.path()).unwrap();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_compute_source_hash_changes_on_file_add() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();

        let hash1 = compute_source_hash(dir.path()).unwrap();

        std::fs::write(dir.path().join("b.txt"), "world").unwrap();

        let hash2 = compute_source_hash(dir.path()).unwrap();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_compute_source_hash_changes_on_rename() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("old_name.txt"), "hello").unwrap();

        let hash1 = compute_source_hash(dir.path()).unwrap();

        std::fs::remove_file(dir.path().join("old_name.txt")).unwrap();
        std::fs::write(dir.path().join("new_name.txt"), "hello").unwrap();

        let hash2 = compute_source_hash(dir.path()).unwrap();
        assert_ne!(hash1, hash2, "Rename with same content should change hash");
    }

    #[test]
    fn test_compute_source_hash_no_boundary_ambiguity() {
        // Verify that length-prefixed hashing prevents collisions when
        // path/content byte boundaries differ.
        let dir1 = TempDir::new().unwrap();
        std::fs::write(dir1.path().join("ab"), "cd").unwrap();
        let hash1 = compute_source_hash(dir1.path()).unwrap();

        let dir2 = TempDir::new().unwrap();
        std::fs::write(dir2.path().join("a"), "bcd").unwrap();
        let hash2 = compute_source_hash(dir2.path()).unwrap();

        assert_ne!(
            hash1, hash2,
            "Different path/content splits should produce different hashes"
        );
    }

    #[test]
    fn test_compute_source_hash_ignores_node_modules() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();

        let hash1 = compute_source_hash(dir.path()).unwrap();

        let nm = dir.path().join("node_modules");
        std::fs::create_dir(&nm).unwrap();
        std::fs::write(nm.join("dep.js"), "lots of code").unwrap();

        let hash2 = compute_source_hash(dir.path()).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_compute_source_hash_ignores_target() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn main() {}").unwrap();

        let hash1 = compute_source_hash(dir.path()).unwrap();

        let target = dir.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("output"), "binary data").unwrap();

        let hash2 = compute_source_hash(dir.path()).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_compute_source_hash_ignores_dist() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("src.ts"), "export {}").unwrap();

        let hash1 = compute_source_hash(dir.path()).unwrap();

        let dist = dir.path().join("dist");
        std::fs::create_dir(&dist).unwrap();
        std::fs::write(dist.join("bundle.js"), "compiled").unwrap();

        let hash2 = compute_source_hash(dir.path()).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_compute_source_hash_ignores_git() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("readme.md"), "# Plugin").unwrap();

        let hash1 = compute_source_hash(dir.path()).unwrap();

        let git = dir.path().join(".git");
        std::fs::create_dir(&git).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main").unwrap();

        let hash2 = compute_source_hash(dir.path()).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_compute_source_hash_ignores_wasm_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.ts"), "export {}").unwrap();

        let hash1 = compute_source_hash(dir.path()).unwrap();

        std::fs::write(dir.path().join("plugin.wasm"), "wasm binary").unwrap();

        let hash2 = compute_source_hash(dir.path()).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_compute_source_hash_ignores_bridge_script() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.js"), "module.exports = {}").unwrap();

        let hash1 = compute_source_hash(dir.path()).unwrap();

        std::fs::write(dir.path().join("astrid_bridge.mjs"), "bridge code").unwrap();

        let hash2 = compute_source_hash(dir.path()).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_collect_source_paths_nested() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("main.ts"), "export {}").unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();

        let mut paths = Vec::new();
        collect_source_paths(dir.path(), &mut paths).unwrap();
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn test_collect_source_paths_skips_all_ignored() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.ts"), "code").unwrap();

        // Create all ignored directories with files
        for ignored in IGNORED_DIRS {
            let d = dir.path().join(ignored);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("file.txt"), "data").unwrap();
        }

        let mut paths = Vec::new();
        collect_source_paths(dir.path(), &mut paths).unwrap();
        assert_eq!(paths.len(), 1); // Only index.ts
    }

    #[test]
    fn test_is_in_ignored_dir() {
        assert!(is_in_ignored_dir(Path::new("/foo/node_modules/bar.js")));
        assert!(is_in_ignored_dir(Path::new("/project/target/debug/binary")));
        assert!(is_in_ignored_dir(Path::new("/app/dist/bundle.js")));
        assert!(is_in_ignored_dir(Path::new("/repo/.git/HEAD")));
        assert!(!is_in_ignored_dir(Path::new("/src/main.ts")));
        assert!(!is_in_ignored_dir(Path::new("/plugin/index.js")));
    }

    #[test]
    fn test_compute_source_hash_empty_dir() {
        let dir = TempDir::new().unwrap();
        let hash = compute_source_hash(dir.path()).unwrap();
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64); // blake3 hex is 64 chars
    }

    #[test]
    fn test_compute_source_hash_with_subdirs() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("lib");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("utils.ts"), "export const x = 1;").unwrap();
        std::fs::write(dir.path().join("index.ts"), "import './lib/utils';").unwrap();

        let hash = compute_source_hash(dir.path()).unwrap();
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn test_watcher_config_default() {
        let config = WatcherConfig::default();
        assert!(config.watch_paths.is_empty());
        assert_eq!(config.debounce, DEFAULT_DEBOUNCE);
    }

    #[tokio::test]
    async fn test_watcher_creation() {
        let dir = TempDir::new().unwrap();
        let config = WatcherConfig {
            watch_paths: vec![dir.path().to_path_buf()],
            ..Default::default()
        };

        let result = PluginWatcher::new(config);
        assert!(result.is_ok());
    }

    /// Test the hash deduplication logic directly: first call should emit,
    /// second call with same content should not.
    #[tokio::test]
    async fn test_process_plugin_change_deduplicates() {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path().join("my-plugin");
        std::fs::create_dir(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "id = \"my-plugin\"\nname = \"Test\"\nversion = \"0.1.0\"\n\n[entry_point]\ntype = \"wasm\"\npath = \"plugin.wasm\"\n",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("index.ts"), "export {}").unwrap();

        let config = WatcherConfig {
            watch_paths: vec![dir.path().to_path_buf()],
            debounce: Duration::from_millis(50),
        };
        let (mut watcher, mut events) = PluginWatcher::new(config).unwrap();

        // First call: should emit an event (no cached hash).
        watcher.process_plugin_change(&plugin_dir).await;
        let event = events.try_recv();
        assert!(event.is_ok(), "First call should emit an event");
        match event.unwrap() {
            WatchEvent::PluginChanged {
                plugin_dir: pd,
                source_hash,
            } => {
                assert_eq!(pd, plugin_dir);
                assert_eq!(source_hash.len(), 64);
            },
            WatchEvent::Error(e) => panic!("Unexpected error: {e}"),
        }

        // Second call with same content: should NOT emit (hash cached).
        watcher.process_plugin_change(&plugin_dir).await;
        let event = events.try_recv();
        assert!(
            event.is_err(),
            "Second call with same content should not emit"
        );

        // Modify content: should emit again.
        std::fs::write(plugin_dir.join("index.ts"), "export const x = 1;").unwrap();
        watcher.process_plugin_change(&plugin_dir).await;
        let event = events.try_recv();
        assert!(event.is_ok(), "Changed content should emit a new event");
    }

    /// Test that `resolve_plugin_dir` correctly identifies plugin roots.
    #[test]
    fn test_resolve_plugin_dir() {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path().join("my-plugin");
        std::fs::create_dir(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "id = \"test\"\nname = \"T\"\nversion = \"0.1.0\"\n\n[entry_point]\ntype = \"wasm\"\npath = \"p.wasm\"\n",
        )
        .unwrap();
        let src = plugin_dir.join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("main.ts"), "export {}").unwrap();

        let config = WatcherConfig {
            watch_paths: vec![dir.path().to_path_buf()],
            ..Default::default()
        };
        let (watcher, _events) = PluginWatcher::new(config).unwrap();

        // File inside plugin dir resolves to plugin root.
        let resolved = watcher.resolve_plugin_dir(&src.join("main.ts"));
        assert_eq!(resolved, Some(plugin_dir.clone()));

        // File at watch root with no manifest resolves to None.
        let resolved = watcher.resolve_plugin_dir(&dir.path().join("random.txt"));
        assert!(resolved.is_none());
    }

    /// Test that `resolve_plugin_dir` finds `OpenClaw` plugins too.
    #[test]
    fn test_resolve_plugin_dir_openclaw() {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path().join("oc-plugin");
        std::fs::create_dir(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("openclaw.plugin.json"),
            r#"{"id":"test","configSchema":{}}"#,
        )
        .unwrap();
        std::fs::write(plugin_dir.join("src/index.ts"), "").ok(); // ignore if src doesn't exist
        std::fs::write(plugin_dir.join("index.ts"), "export {}").unwrap();

        let config = WatcherConfig {
            watch_paths: vec![dir.path().to_path_buf()],
            ..Default::default()
        };
        let (watcher, _events) = PluginWatcher::new(config).unwrap();

        let resolved = watcher.resolve_plugin_dir(&plugin_dir.join("index.ts"));
        assert_eq!(resolved, Some(plugin_dir));
    }

    /// Integration test: verify the watcher detects real filesystem changes.
    /// Marked `#[ignore = "flaky on CI due to filesystem timing"]` because FSEvents/inotify latency makes it flaky in
    /// CI and sandboxed environments. Run manually with `--ignored`.
    #[tokio::test]
    #[ignore = "flaky on CI due to filesystem timing"]
    async fn test_watcher_integration_real_fs() {
        let dir = TempDir::new().unwrap();
        let plugin_dir = dir.path().join("my-plugin");
        std::fs::create_dir(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "id = \"my-plugin\"\nname = \"Test\"\nversion = \"0.1.0\"\n\n[entry_point]\ntype = \"wasm\"\npath = \"plugin.wasm\"\n",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("index.ts"), "export {}").unwrap();

        let config = WatcherConfig {
            watch_paths: vec![dir.path().to_path_buf()],
            debounce: Duration::from_millis(100),
        };

        let (watcher, mut events) = PluginWatcher::new(config).unwrap();
        let handle = tokio::spawn(async move { watcher.run().await });

        // Wait for watcher to initialize (FSEvents needs warm-up).
        tokio::time::sleep(Duration::from_secs(2)).await;
        std::fs::write(plugin_dir.join("index.ts"), "export const x = 1;").unwrap();

        let event = tokio::time::timeout(Duration::from_secs(10), events.recv()).await;
        assert!(event.is_ok(), "Should receive an event within timeout");

        handle.abort();
    }
}
