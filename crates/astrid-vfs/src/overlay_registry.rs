//! Per-principal [`OverlayVfs`] registry.
//!
//! Layer 4 of multi-tenancy (issue #668): each principal invoking a capsule
//! gets its own [`OverlayVfs`](crate::OverlayVfs) on top of the shared
//! workspace. The lower layer (the read-only workspace) is common, but
//! every principal writes into an isolated upper layer backed by a fresh
//! [`tempfile::TempDir`]. Two principals writing `foo.txt` never see each
//! other's bytes.
//!
//! The registry is lazy — overlays are built on first use and cached for
//! the kernel's lifetime. It is also bounded: a single process cannot grow
//! the upper-layer tempdir count without bound, so when the registry hits
//! its principal cap the least-recently-used idle entry is evicted. Entries
//! that have been touched within the idle threshold are retained even if
//! they are over the cap — the cap is a soft admission control, not a hard
//! cap — to avoid churning hot overlays on a near-full registry.
//!
//! Revocation / commit semantics are unchanged: [`OverlayVfs::commit`] and
//! [`OverlayVfs::rollback`] are not called from any production path today;
//! the registry simply stands up the data-structure isolation required by
//! invariant #7 from issue #653.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use astrid_capabilities::DirHandle;
use astrid_core::principal::PrincipalId;

use crate::{HostVfs, OverlayVfs};

/// Default cap on the number of principals cached. Exposed as an env var so
/// multi-tenant operators can tune it without recompiling.
const DEFAULT_MAX_PRINCIPALS: usize = 1024;

/// Environment variable tuning the principal cap.
pub const ENV_MAX_PRINCIPALS: &str = "ASTRID_OVERLAY_REGISTRY_MAX_PRINCIPALS";

/// How long an overlay must be idle before it is eligible for eviction when
/// the registry is at or above its cap.
const DEFAULT_IDLE_EVICTION: Duration = Duration::from_mins(10);

/// Read the configured cap from the environment, clamping to sensible
/// bounds. The env var is read at registry construction — hot-reload is
/// deliberately out of scope.
fn resolve_max_principals() -> usize {
    std::env::var(ENV_MAX_PRINCIPALS)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n: &usize| n >= 1)
        .unwrap_or(DEFAULT_MAX_PRINCIPALS)
}

/// A cached overlay bundle for a single principal.
///
/// The upper-layer `TempDir` lives **inside** the `OverlayVfs` (via
/// [`OverlayVfs::new_with_upper_guard`]), not in this struct. Evicting the
/// entry therefore removes the cache slot without deleting the physical
/// tempdir — any concurrent task that still holds an `Arc<OverlayVfs>`
/// clone keeps the tempdir alive, and the directory is only unlinked when
/// the last clone is dropped.
struct Entry {
    overlay: Arc<OverlayVfs>,
    /// Milliseconds since [`OverlayVfsRegistry::anchor`] at the last cache hit.
    ///
    /// Stored atomically so cache-hit updates can happen under a read lock —
    /// the resolve hot path is every WASM invocation and must not serialise
    /// unrelated principals on a write lock. Eviction (write-lock path)
    /// reads each entry's value with [`Ordering::Relaxed`] — stale reads are
    /// fine because the only consequence is picking a slightly different
    /// LRU victim.
    last_used_ms: AtomicU64,
}

/// Lazy, bounded, process-lifetime cache of per-principal [`OverlayVfs`]
/// instances.
pub struct OverlayVfsRegistry {
    workspace_root: PathBuf,
    root_handle: DirHandle,
    max_principals: usize,
    idle_eviction: Duration,
    /// Reference point for every entry's [`Entry::last_used_ms`]. Fixed at
    /// construction so timestamps remain monotonic across the registry's
    /// lifetime.
    anchor: Instant,
    overlays: RwLock<HashMap<PrincipalId, Entry>>,
}

impl OverlayVfsRegistry {
    /// Create a new registry rooted at `workspace_root` with the
    /// capability `root_handle`.
    ///
    /// The cap on concurrent principals is read from
    /// `ASTRID_OVERLAY_REGISTRY_MAX_PRINCIPALS` (default 1024).
    #[must_use]
    pub fn new(workspace_root: PathBuf, root_handle: DirHandle) -> Self {
        Self {
            workspace_root,
            root_handle,
            max_principals: resolve_max_principals(),
            idle_eviction: DEFAULT_IDLE_EVICTION,
            anchor: Instant::now(),
            overlays: RwLock::new(HashMap::new()),
        }
    }

    /// Create a registry with an explicit cap and idle-eviction window.
    /// Primarily for tests — production callers use [`Self::new`].
    #[must_use]
    pub fn with_limits(
        workspace_root: PathBuf,
        root_handle: DirHandle,
        max_principals: usize,
        idle_eviction: Duration,
    ) -> Self {
        Self {
            workspace_root,
            root_handle,
            max_principals: max_principals.max(1),
            idle_eviction,
            anchor: Instant::now(),
            overlays: RwLock::new(HashMap::new()),
        }
    }

    /// Milliseconds elapsed since the registry's [`anchor`](Self::anchor),
    /// clamped to `u64::MAX`. Monotonic for the registry's lifetime.
    fn now_ms(&self) -> u64 {
        u64::try_from(self.anchor.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Number of principals currently cached. Test-only introspection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.overlays
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Whether the registry has any cached overlays. Present to satisfy
    /// clippy's `len_without_is_empty` lint — not a hot path.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Resolve the overlay for `principal`, populating on first use.
    ///
    /// Subsequent calls return the cached `Arc<OverlayVfs>` clone and
    /// update the entry's `last_used_ms` timestamp atomically under a read
    /// lock, so concurrent resolves for *different* principals do not
    /// serialise on a shared write lock (the WASM invocation hot path).
    ///
    /// # Errors
    ///
    /// Returns an IO error if the tempdir cannot be created or the VFS
    /// mounts cannot be registered. Callers should deny the invocation
    /// (fail-closed) rather than fall back to a shared overlay.
    pub async fn resolve(&self, principal: &PrincipalId) -> std::io::Result<Arc<OverlayVfs>> {
        // Fast path: cache hit under a read lock. The timestamp bump uses
        // atomic store so the read-lock path is enough — no writer contention
        // for unrelated principals.
        {
            let guard = self
                .overlays
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(entry) = guard.get(principal) {
                entry.last_used_ms.store(self.now_ms(), Ordering::Relaxed);
                return Ok(Arc::clone(&entry.overlay));
            }
        }

        // Slow path: build a fresh overlay before re-acquiring the write
        // lock. Building under the lock would serialize concurrent
        // first-access for different principals; building outside lets them
        // parallelise at the cost of a rare duplicate-build on the same
        // principal (handled in the insertion step).
        let overlay = self.build_for(principal).await?;

        let mut guard = self
            .overlays
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Race: another task may have inserted an entry for `principal`
        // while we were building. First writer wins; we throw away ours.
        if let Some(existing) = guard.get(principal) {
            existing
                .last_used_ms
                .store(self.now_ms(), Ordering::Relaxed);
            return Ok(Arc::clone(&existing.overlay));
        }

        if guard.len() >= self.max_principals {
            self.evict_idle_locked(&mut guard);
        }

        let entry = Entry {
            overlay: Arc::clone(&overlay),
            last_used_ms: AtomicU64::new(self.now_ms()),
        };
        guard.insert(principal.clone(), entry);
        Ok(overlay)
    }

    /// Drop the cached overlay for `principal`, if any. Useful for tests
    /// and for future management IPC that revokes a principal's access.
    pub fn invalidate(&self, principal: &PrincipalId) {
        let _ = self
            .overlays
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(principal);
    }

    /// Build a fresh overlay for `principal`. Lower layer = workspace,
    /// upper layer = a new tempdir whose lifetime is bound to the returned
    /// `Arc<OverlayVfs>` — the tempdir is deleted only when the last Arc
    /// clone is dropped, so an in-flight capsule invocation cannot have its
    /// upper layer yanked out from under it by a concurrent registry
    /// eviction.
    async fn build_for(&self, principal: &PrincipalId) -> std::io::Result<Arc<OverlayVfs>> {
        let lower = HostVfs::new();
        lower
            .register_dir(self.root_handle.clone(), self.workspace_root.clone())
            .await
            .map_err(|e| {
                std::io::Error::other(format!(
                    "overlay registry: register lower dir for {principal}: {e:?}"
                ))
            })?;

        let upper_dir = tempfile::Builder::new()
            .prefix(&format!("astrid-overlay-{principal}-"))
            .tempdir()
            .map_err(|e| {
                std::io::Error::other(format!(
                    "overlay registry: create upper tempdir for {principal}: {e}"
                ))
            })?;

        let upper = HostVfs::new();
        upper
            .register_dir(self.root_handle.clone(), upper_dir.path().to_path_buf())
            .await
            .map_err(|e| {
                std::io::Error::other(format!(
                    "overlay registry: register upper dir for {principal}: {e:?}"
                ))
            })?;

        Ok(Arc::new(OverlayVfs::new_with_upper_guard(
            Box::new(lower),
            Box::new(upper),
            Arc::new(upper_dir),
        )))
    }

    /// Evict the single oldest entry whose `last_used_ms` is beyond the
    /// idle-eviction window. If no entry is idle, evict the globally
    /// oldest — the cap is a hard bound on tempdir allocations, not a
    /// soft heuristic.
    fn evict_idle_locked(&self, guard: &mut HashMap<PrincipalId, Entry>) {
        let now_ms = self.now_ms();
        let idle_cutoff_ms = u64::try_from(self.idle_eviction.as_millis()).unwrap_or(u64::MAX);
        let cutoff_ms = now_ms.saturating_sub(idle_cutoff_ms);

        // Single pass. Tuple-sort `(non_idle_flag, ts)` so idle entries
        // (`ts <= cutoff_ms` → `false`) sort before non-idle ones, and
        // within each group the smallest `ts` wins — i.e. the oldest idle
        // entry is preferred, and if none are idle the globally oldest is
        // picked. Avoids the per-eviction snapshot `Vec` allocation.
        let victim = guard
            .iter()
            .map(|(p, e)| (p, e.last_used_ms.load(Ordering::Relaxed)))
            .min_by_key(|&(_, ts)| (ts > cutoff_ms, ts))
            .map(|(p, _)| p.clone());
        if let Some(p) = victim {
            tracing::info!(principal = %p, "overlay registry at cap, evicting idle entry");
            guard.remove(&p);
        }
    }

    /// Root capability handle used to mount every per-principal overlay.
    ///
    /// Callers must use this handle when invoking [`Vfs`](crate::Vfs) methods
    /// against the overlay returned by [`Self::resolve`] — the overlay only
    /// recognises this handle as the mount root.
    #[must_use]
    pub fn root_handle(&self) -> &DirHandle {
        &self.root_handle
    }
}

impl std::fmt::Debug for OverlayVfsRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OverlayVfsRegistry")
            .field("workspace_root", &self.workspace_root)
            .field("max_principals", &self.max_principals)
            .field("idle_eviction", &self.idle_eviction)
            .field("cached", &self.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "overlay_registry_tests.rs"]
mod tests;
