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

pub mod config;
pub mod config_bridge;
pub mod error;

use std::sync::Arc;
use tokio::sync::RwLock;
use astrid_events::EventBus;
use astrid_capsule::registry::CapsuleRegistry;

/// The core Operating System Kernel.
pub struct Kernel {
    /// The global IPC message bus.
    pub event_bus: Arc<EventBus>,
    /// The process manager (loaded WASM capsules).
    pub plugins: Arc<RwLock<CapsuleRegistry>>,
}

impl Kernel {
    /// Boot a new Kernel instance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            event_bus: Arc::new(EventBus::new()),
            plugins: Arc::new(RwLock::new(CapsuleRegistry::new())),
        }
    }
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}
