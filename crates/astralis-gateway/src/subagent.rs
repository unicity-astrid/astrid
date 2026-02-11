//! Subagent pool management — re-exported from `astralis-runtime`.
//!
//! The canonical implementation lives in [`astralis_runtime::subagent`].
//! This module re-exports all types for backward compatibility.

pub use astralis_runtime::subagent::{
    SubAgentHandle, SubAgentId, SubAgentPool, SubAgentPoolStats, SubAgentStatus,
};
