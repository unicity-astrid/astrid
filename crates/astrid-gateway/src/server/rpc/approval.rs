//! Approval and elicitation RPC method implementations.

use astrid_core::{ApprovalDecision, ElicitationResponse, SessionId};
use jsonrpsee::types::ErrorObjectOwned;
use tracing::info;

use super::RpcImpl;
use crate::rpc::{DaemonEvent, error_codes};

impl RpcImpl {
    pub(super) async fn approval_response_impl(
        &self,
        session_id: SessionId,
        request_id: String,
        decision: ApprovalDecision,
    ) -> Result<(), ErrorObjectOwned> {
        // Look up the session handle (brief read lock).
        // No session mutex needed -- the frontend's pending_approvals has its own lock.
        let handle = {
            let sessions = self.sessions.read().await;
            sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?
        };

        if !handle
            .frontend
            .resolve_approval(&request_id, decision)
            .await
        {
            return Err(ErrorObjectOwned::owned(
                error_codes::INVALID_REQUEST,
                format!("No pending approval with id: {request_id}"),
                None::<()>,
            ));
        }

        Ok(())
    }

    pub(super) async fn elicitation_response_impl(
        &self,
        session_id: SessionId,
        request_id: String,
        response: ElicitationResponse,
    ) -> Result<(), ErrorObjectOwned> {
        // Look up the session handle (brief read lock).
        let handle = {
            let sessions = self.sessions.read().await;
            sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?
        };

        if !handle
            .frontend
            .resolve_elicitation(&request_id, response)
            .await
        {
            return Err(ErrorObjectOwned::owned(
                error_codes::INVALID_REQUEST,
                format!("No pending elicitation with id: {request_id}"),
                None::<()>,
            ));
        }

        Ok(())
    }

    pub(super) async fn cancel_turn_impl(
        &self,
        session_id: SessionId,
    ) -> Result<(), ErrorObjectOwned> {
        let handle = {
            let sessions = self.sessions.read().await;
            sessions.get(&session_id).cloned().ok_or_else(|| {
                ErrorObjectOwned::owned(
                    error_codes::SESSION_NOT_FOUND,
                    format!("Session not found: {session_id}"),
                    None::<()>,
                )
            })?
        };

        // Take the turn handle (if a turn is running) and abort it.
        let join_handle = handle.turn_handle.lock().await.take();
        if let Some(jh) = join_handle {
            jh.abort();
            let _ = handle.event_tx.send(DaemonEvent::TurnComplete);
            info!(session_id = %session_id, "Turn cancelled via RPC");
        }

        Ok(())
    }
}
