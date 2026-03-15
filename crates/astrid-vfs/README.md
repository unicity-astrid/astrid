# astrid-vfs

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The agent operates against a copy of the filesystem. The workspace is protected until you say otherwise.**

In the OS model, this is the kernel's virtual filesystem layer. Agents read from the workspace. Writes go into an ephemeral upper layer backed by a temp directory. Session ends: commit the diff to the workspace, or drop the temp directory to discard everything. The agent never touches the real files until a human approves the commit.

## Two sandboxes, two layers

**Path sandbox.** `path::resolve_path` performs a purely lexical traversal check before any syscall. `../../etc/passwd` is rejected at this layer. Below that, `cap-std` enforces OS-level directory confinement. Holding a path string grants nothing. Every operation requires an opaque `DirHandle` or `FileHandle` issued by `astrid-capabilities`.

**Copy-on-write overlay.** `OverlayVfs` stacks a read-write upper layer over a read-only lower layer. Reads fall through: if the file exists in upper, serve it; otherwise read from lower. Writes go strictly to upper. `dirty_paths()` returns what changed. `commit()` propagates dirty files to lower, creating parent directories as needed. `rollback()` discards the upper layer.

## Key design decisions

**Transparent copy-up.** Opening a lower-layer file for writing copies its content to the upper layer first. A per-path `Mutex` prevents duplicate copy-ups under concurrent access. After acquiring the lock, the code re-checks whether another task already completed the copy. Truncate-on-open skips the copy entirely.

**Resource quotas.** 64 concurrent open file handles, enforced via a Tokio semaphore acquired before calling the OS. 50 MB max file size on reads, copy-ups, and commits. Protects against OOM from large files.

**Dirty tracking.** `OverlayVfs` tracks written paths with their kind (`File` or `Dir`). `commit()` handles them differently: files are read-then-written, directories are created. `rollback()` unlinks files and relies on TempDir drop for directory cleanup. `open_dir` eagerly creates mirrored directories in upper for handle mapping but does not record them as dirty.

**`.astridignore` boundary.** The internal `WorktreeVfs` (not yet wired into the main VFS path) denies paths matched by gitignore-syntax rules, protecting host secrets like `.env` files. The boundary module and worktree module exist but are marked `pub(crate)` and `#[allow(dead_code)]`. They are built, not yet integrated.

## Usage

```toml
[dependencies]
astrid-vfs = { workspace = true }
```

```rust
use astrid_vfs::{HostVfs, OverlayVfs, Vfs};
use astrid_capabilities::DirHandle;
use std::path::PathBuf;

// Host-backed sandbox
let vfs = HostVfs::new();
let handle = DirHandle::new();
vfs.register_dir(handle.clone(), PathBuf::from("/var/astrid/sandbox")).await?;

let fh = vfs.open(&handle, "output.txt", true, false).await?;
vfs.write(&fh, b"result").await?;
vfs.close(&fh).await?;

// Copy-on-write overlay
let lower = Box::new(HostVfs::new());
let upper = Box::new(HostVfs::new());
let overlay = OverlayVfs::new(lower, upper);
// All writes go to upper. commit() propagates to lower. rollback() discards.
```

## Known limitations

- Deleting a file that exists only in the lower layer returns `VfsError::NotSupported`. No whiteout support.
- `open_dir` eagerly creates mirrored directories in upper but does not record them as dirty.
- `boundary` and `worktree` modules are built but not yet integrated into the public VFS path.

`#![deny(unsafe_code)]` is enforced crate-wide.

## Development

```bash
cargo test -p astrid-vfs
```

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
