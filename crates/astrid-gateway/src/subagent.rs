//! Subagent pool management — re-exported from `astrid-runtime`.
//!
//! The canonical implementation lives in [`astrid_runtime::subagent`].
//! This module re-exports all types for backward compatibility.

pub use astrid_runtime::subagent::{
    SubAgentHandle, SubAgentId, SubAgentPool, SubAgentPoolStats, SubAgentStatus,
};
