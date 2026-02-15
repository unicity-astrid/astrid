//! Astrid Approval - Human-in-the-loop approval system.
//!
//! This crate provides types and logic for the approval workflow that gates
//! sensitive agent operations behind explicit human confirmation.
//!
//! # Phase 2 Components
//!
//! - **2.1 Approval Types** (this phase): [`SensitiveAction`], [`RiskAssessment`],
//!   [`ApprovalRequest`], [`ApprovalDecision`], [`ApprovalResponse`]
//! - **2.2 Allowance System**: [`Allowance`], [`AllowancePattern`], `AllowanceStore`
//! - **2.3 Approval Manager**: Orchestrates the full approval flow
//! - **2.4 Budget Tracking**: Session and per-action spending limits
//! - **2.5 Security Policy**: Hard boundaries (blocked/approval-required tools)
//! - **2.6 Security Interceptor**: Combines all layers (intersection semantics)
//!
//! # Relationship to Frontend Types
//!
//! The approval types in this crate are the **internal** representation used by
//! the security system. The types in [`astrid_core::frontend`] are the
//! **UI-facing** types that frontends render. The approval manager converts
//! between them when presenting requests to users.
//!
//! # Example
//!
//! ```
//! use astrid_approval::{SensitiveAction, ApprovalRequest, ApprovalDecision, RiskAssessment};
//! use astrid_core::types::RiskLevel;
//!
//! // Classify a risky action
//! let action = SensitiveAction::FileDelete {
//!     path: "/home/user/important.txt".to_string(),
//! };
//!
//! // Create a request with context
//! let request = ApprovalRequest::new(action, "Cleaning up temporary files");
//! assert_eq!(request.assessment.level, RiskLevel::High);
//!
//! // Decisions
//! let approved = ApprovalDecision::Approve;
//! assert!(approved.is_approved());
//!
//! let denied = ApprovalDecision::Deny { reason: "Too risky".to_string() };
//! assert!(!denied.is_approved());
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

pub mod prelude;

pub mod action;
pub mod allowance;
pub mod budget;
pub mod deferred;
pub mod interceptor;
pub mod manager;
pub mod policy;
pub mod request;

pub use action::SensitiveAction;
pub use allowance::{Allowance, AllowanceId, AllowancePattern, AllowanceStore};
pub use budget::{
    BudgetConfig, BudgetResult, BudgetTracker, WorkspaceBudgetSnapshot, WorkspaceBudgetTracker,
};
pub use deferred::{
    ActionContext, DeferredResolution, DeferredResolutionStore, FallbackBehavior, PendingAction,
    Priority, ResolutionId,
};
pub use interceptor::{BudgetWarning, InterceptProof, InterceptResult, SecurityInterceptor};
pub use manager::{ApprovalHandler, ApprovalManager, ApprovalOutcome, ApprovalProof};
pub use policy::{PolicyResult, SecurityPolicy};
pub use request::{ApprovalDecision, ApprovalRequest, ApprovalResponse, RequestId, RiskAssessment};
