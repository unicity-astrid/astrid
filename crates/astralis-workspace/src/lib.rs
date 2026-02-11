//! Astralis Workspace - Operational boundaries for agent actions.
//!
//! This crate provides workspace boundaries that define where the agent
//! can operate. Unlike the WASM sandbox (which is inescapable), the
//! operational workspace can be escaped with user approval.
//!
//! # Key Concepts
//!
//! - **Workspace**: A directory tree where the agent can freely operate
//! - **Escape**: Operations outside the workspace require approval
//! - **Modes**: Safe (always ask), Guided (smart defaults), Autonomous (no restrictions)
//!
//! # Example
//!
//! ```rust,ignore
//! use astralis_workspace::{WorkspaceBoundary, WorkspaceConfig, WorkspaceMode};
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

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

pub mod prelude;

pub mod boundaries;
pub mod config;
pub mod escape;
pub mod profiles;

pub use boundaries::{PathCheck, WorkspaceBoundary};
pub use config::{EscapePolicy, WorkspaceConfig, WorkspaceMode};
pub use escape::{EscapeDecision, EscapeRequest};
pub use profiles::WorkspaceProfile;
