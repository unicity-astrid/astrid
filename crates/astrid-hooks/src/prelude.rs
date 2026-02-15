//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astrid_hooks::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust,ignore
//! use astrid_hooks::prelude::*;
//!
//! let mut manager = HookManager::new();
//!
//! let hook = Hook::new(HookEvent::PreToolCall)
//!     .with_handler(HookHandler::Command {
//!         command: "echo".to_string(),
//!         args: vec!["Tool called: $TOOL_NAME".to_string()],
//!         env: Default::default(),
//!     });
//!
//! manager.register(hook);
//! ```

// Core hook types
pub use crate::{Hook, HookEvent, HookHandler};

// Manager and executor
pub use crate::{HookExecutor, HookManager};

// Configuration
pub use crate::HooksConfig;

// Discovery
pub use crate::discover_hooks;

// Profiles
pub use crate::HookProfile;

// Result type
pub use crate::HookResult;
