//! CLI approval handler — skeleton implementation.
//!
//! Implements [`ApprovalHandler`] for the CLI frontend using `dialoguer`
//! for bare-bones terminal prompting. This is a minimal integration proving
//! the trait wires cleanly; the full UI (colors, box drawing, progress) will
//! be implemented when the CLI frontend is reworked.

use astralis_approval::prelude::*;
use async_trait::async_trait;
use dialoguer::{Select, theme::ColorfulTheme};

/// CLI implementation of the approval handler.
///
/// Presents approval requests via terminal prompts and returns the user's
/// decision. Supports all four approval options: Once, Session, Always, Deny.
pub struct CliApprovalHandler;

impl CliApprovalHandler {
    /// Create a new CLI approval handler.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for CliApprovalHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ApprovalHandler for CliApprovalHandler {
    async fn request_approval(&self, request: ApprovalRequest) -> Option<ApprovalResponse> {
        // Show the request
        println!();
        println!("--- Approval Required ---");
        println!("  Action:  {}", request.action);
        println!("  Risk:    {}", request.assessment);
        println!("  Context: {}", request.context);
        println!("-------------------------");

        let options = &[
            "Approve (once)",
            "Approve (session)",
            "Approve (workspace)",
            "Allow Always (1h capability)",
            "Deny",
        ];

        let selection = Select::with_theme(&ColorfulTheme::default())
            .items(options)
            .default(0)
            .interact()
            .ok()?;

        let decision = match selection {
            0 => ApprovalDecision::Approve,
            1 => ApprovalDecision::ApproveSession,
            2 => ApprovalDecision::ApproveWorkspace,
            3 => ApprovalDecision::ApproveAlways,
            4 => ApprovalDecision::Deny {
                reason: "denied by user".to_string(),
            },
            _ => return None,
        };

        Some(ApprovalResponse::new(request.id, decision))
    }

    fn is_available(&self) -> bool {
        // CLI is available whenever the process is running
        true
    }
}
