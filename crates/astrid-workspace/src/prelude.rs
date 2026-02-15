//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astrid_workspace::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust,ignore
//! use astrid_workspace::prelude::*;
//!
//! let config = WorkspaceConfig::new("/home/user/project")
//!     .with_mode(WorkspaceMode::Guided);
//!
//! let boundary = WorkspaceBoundary::new(config);
//!
//! // Check if a path is allowed
//! match boundary.check("/home/user/project/src/main.rs") {
//!     PathCheck::Allowed => println!("Path is in workspace"),
//!     PathCheck::RequiresApproval => println!("Needs user approval"),
//!     _ => {}
//! }
//! ```

// Boundary checking
pub use crate::{PathCheck, WorkspaceBoundary};

// Configuration
pub use crate::{EscapePolicy, WorkspaceConfig, WorkspaceMode};

// Escape handling
pub use crate::{EscapeDecision, EscapeRequest};

// Profiles
pub use crate::WorkspaceProfile;
