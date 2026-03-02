#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![allow(clippy::module_name_repetitions)]

//! Astrid Kernel - The core execution engine and IPC router.
//!
//! The Kernel is a pure, decentralized WASM runner. It contains no business
//! logic, no cognitive loops, and no network servers. Its sole responsibility
//! is to instantiate `astrid_events::EventBus`, load `.capsule` files into
//! the Extism sandbox, and route IPC bytes between them.

/// The Unix Domain Socket IPC bridge for multi-process Extism scaling.
pub mod socket;

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use astrid_events::EventBus;
use astrid_capsule::registry::CapsuleRegistry;
use astrid_vfs::{Vfs, OverlayVfs, HostVfs};
use astrid_capabilities::DirHandle;

/// The core Operating System Kernel.
pub struct Kernel {
    /// The global IPC message bus.
    pub event_bus: Arc<EventBus>,
    /// The process manager (loaded WASM capsules).
    pub plugins: Arc<RwLock<CapsuleRegistry>>,
    /// The global Virtual File System mount.
    pub vfs: Arc<dyn Vfs>,
    /// The global physical root handle (cap-std) for the VFS.
    pub vfs_root_handle: DirHandle,
    /// The physical path the VFS is mounted to.
    pub workspace_root: PathBuf,
}

impl Kernel {
    /// Boot a new Kernel instance mounted at the specified directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the VFS mount paths cannot be registered.
    pub async fn new(workspace_root: PathBuf) -> Result<Self, std::io::Error> {
        let event_bus = Arc::new(EventBus::new());
        let plugins = Arc::new(RwLock::new(CapsuleRegistry::new()));

        // 1. Establish the physical security boundary (sandbox handle)
        let root_handle = DirHandle::new();

        // 2. Initialize the physical filesystem layers
        let lower_vfs = HostVfs::new();
        lower_vfs.register_dir(root_handle.clone(), workspace_root.clone()).await.map_err(|_| std::io::Error::other("Failed to register lower vfs dir"))?;

        let upper_vfs = HostVfs::new();
        upper_vfs.register_dir(root_handle.clone(), workspace_root.clone()).await.map_err(|_| std::io::Error::other("Failed to register upper vfs dir"))?;

        // 3. Wrap in copy-on-write OverlayVfs
        let overlay_vfs = OverlayVfs::new(Box::new(lower_vfs), Box::new(upper_vfs));

        // Spawn the local Unix Domain Socket IPC bridge
        std::mem::drop(crate::socket::spawn_socket_server(Arc::clone(&event_bus)));

        Ok(Self {
            event_bus,
            plugins,
            vfs: Arc::new(overlay_vfs),
            vfs_root_handle: root_handle,
            workspace_root,
        })
    }
}

impl Default for Kernel {
    fn default() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        tokio::runtime::Handle::current().block_on(Self::new(root)).expect("Failed to init kernel")
    }
}
