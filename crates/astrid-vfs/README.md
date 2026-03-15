# astrid-vfs

[![Crates.io](https://img.shields.io/crates/v/astrid-vfs)](https://crates.io/crates/astrid-vfs)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)
[![CI](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml/badge.svg)](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml)

Virtual filesystem and capability sandbox for the Astrid agent runtime.

`astrid-vfs` is the storage isolation layer for Astrid agents. It translates opaque capability handles from `astrid-capabilities` into safe, sandboxed host operations, ensuring agents cannot access paths they were not explicitly granted. Path traversal, absolute path injection, and symlink escape are blocked at two independent layers: a lexical path resolver that runs before any syscall, and `cap-std` ambient authority boundaries enforced by the OS. An overlay filesystem with explicit commit and rollback completes the picture, staging all agent writes in a temporary upper layer until the caller decides to persist or discard them.

## Core Features

- **Capability-based access control**: Every filesystem operation requires an opaque `DirHandle` or `FileHandle` issued by `astrid-capabilities`. Holding a path string alone grants nothing.
- **Dual-layer path sandboxing**: `path::resolve_path` performs a purely lexical traversal check before any syscall. `cap-std` then enforces OS-level directory confinement so the kernel itself rejects escapes.
- **Copy-on-write overlay**: `OverlayVfs` stacks a read-write upper layer over a read-only lower layer. Reads fall through to lower when absent in upper. Writes are confined to upper. An explicit `commit()` propagates changes to lower; `rollback()` discards them.
- **Transparent copy-up**: When an agent opens a lower-layer file for writing, the overlay copies its content to the upper layer before the write, preserving the original until commit. Per-path mutex locks prevent duplicate copy-ups under concurrent access.
- **Resource quotas**: `HostVfs` caps concurrent open file handles at 64 via a Tokio semaphore acquired before the OS call. Reads and copy-ups reject files larger than 50 MB to prevent OOM conditions.
- **Dirty tracking**: `OverlayVfs` maintains a set of paths written to the upper layer, distinguishing files from directories. `dirty_paths()`, `commit()`, and `rollback()` operate on this set.
- **`.astridignore` boundary** (internal): `WorktreeVfs` wraps `HostVfs` and denies any path matched by gitignore-syntax rules, protecting host secrets such as `.env` files from agent access even when they are physically present in the worktree.
- **`#![deny(unsafe_code)]`**: The entire crate is safe Rust.

## Quick Start

Add `astrid-vfs` to your `Cargo.toml`:

```toml
[dependencies]
astrid-vfs = "0.2.0"
```

## API Reference

### Key Types

#### `Vfs` trait

The central async trait. All filesystem operations are expressed through it. Implementations are `Send + Sync` and usable from any Tokio runtime.

```rust
#[async_trait]
pub trait Vfs: Send + Sync {
    async fn exists(&self, handle: &DirHandle, path: &str) -> VfsResult<bool>;
    async fn readdir(&self, handle: &DirHandle, path: &str) -> VfsResult<Vec<VfsDirEntry>>;
    async fn stat(&self, handle: &DirHandle, path: &str) -> VfsResult<VfsMetadata>;
    async fn mkdir(&self, handle: &DirHandle, path: &str) -> VfsResult<()>;
    async fn unlink(&self, handle: &DirHandle, path: &str) -> VfsResult<()>;
    async fn open(&self, handle: &DirHandle, path: &str, write: bool, truncate: bool) -> VfsResult<FileHandle>;
    async fn open_dir(&self, handle: &DirHandle, path: &str, new_handle: DirHandle) -> VfsResult<()>;
    async fn close_dir(&self, handle: &DirHandle) -> VfsResult<()>;
    async fn read(&self, handle: &FileHandle) -> VfsResult<Vec<u8>>;
    async fn write(&self, handle: &FileHandle, content: &[u8]) -> VfsResult<()>;
    async fn close(&self, handle: &FileHandle) -> VfsResult<()>;
}
```

#### `HostVfs`

Backed by the physical host filesystem via `cap-std`. Call `register_dir` to bind a `DirHandle` to a physical path; after that, all operations through that handle are OS-sandboxed to that directory tree.

```rust
let vfs = HostVfs::new();

// The daemon grants a capability to a specific physical path.
let root_handle = DirHandle::new();
vfs.register_dir(root_handle.clone(), PathBuf::from("/var/astrid/sandbox/agent-1"))
    .await?;

// Open a file for writing within the sandbox.
let fh = vfs.open(&root_handle, "output.txt", true, false).await?;
vfs.write(&fh, b"result").await?;
vfs.close(&fh).await?;
```

#### `OverlayVfs`

Composes two `Vfs` implementations into a copy-on-write stack. The lower layer is treated as read-only from the overlay's perspective; all mutations land in the upper layer.

```rust
let lower = Box::new(HostVfs::new()); // read-only workspace
let upper = Box::new(HostVfs::new()); // ephemeral temp dir
let overlay = OverlayVfs::new(lower, upper);

// Register handles on both underlying VFS instances before use.

// All writes go to upper.
let fh = overlay.open(&handle, "draft.txt", true, true).await?;
overlay.write(&fh, b"changes").await?;
overlay.close(&fh).await?;

// Inspect what is staged.
let staged: Vec<String> = overlay.dirty_paths();

// Persist to lower.
let committed: Vec<String> = overlay.commit(&handle).await?;

// Or discard.
let discarded: Vec<String> = overlay.rollback(&handle).await?;
```

#### `VfsMetadata`

Returned by `stat`. Fields: `is_dir: bool`, `is_file: bool`, `size: u64`, `mtime: u64` (seconds since Unix epoch). Implements `Serialize` and `Deserialize`.

#### `VfsDirEntry`

Returned by `readdir`. Fields: `name: String`, `is_dir: bool`. Implements `Serialize` and `Deserialize`.

#### `VfsError`

```rust
pub enum VfsError {
    SandboxViolation(String), // path traversal or absolute path rejected
    InvalidHandle,            // unrecognized or already-closed handle
    Io(std::io::Error),       // underlying OS error
    NotFound(String),
    PermissionDenied(String), // boundary rule, FD quota, or size limit
    NotSupported(String),     // operation not implemented by this layer
}
```

### `path::resolve_path`

```rust
pub fn resolve_path(base_root: &Path, request_path: &str) -> VfsResult<PathBuf>
```

Purely lexical traversal check. Rejects absolute paths and any `..` sequence that would climb above `base_root`. Does not touch the filesystem. Returns `VfsError::SandboxViolation` on any violation.

## Architecture

The crate has three concrete `Vfs` implementations, layered from lowest to highest trust:

| Type | Visibility | Purpose |
|------|-----------|---------|
| `HostVfs` | `pub` | Physical disk, sandboxed by `cap-std` |
| `OverlayVfs` | `pub` | CoW staging layer over any two `Vfs` impls |
| `WorktreeVfs` | `pub(crate)` | `HostVfs` + `.astridignore` deny rules for git worktrees |

`OverlayVfs::open_dir` eagerly creates the mirrored directory in the upper layer to maintain symmetric handle mappings across both layers, but does not record those structural directories as dirty. Only explicit `mkdir` calls and file writes reach the dirty set.

Deleting a file that exists only in the lower layer returns `VfsError::NotSupported` because whiteout support is not implemented. Only upper-layer files can be unlinked.

## Development

```bash
cargo test -p astrid-vfs
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
