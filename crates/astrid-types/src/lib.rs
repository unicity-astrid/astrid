//! Shared data types for the Astrid secure agent runtime.
//!
//! This crate provides the canonical definitions for:
//! - IPC payload schemas (cross-boundary messaging between WASM guests and host)
//! - LLM message, tool, and streaming types
//! - Kernel management API request/response types
//!
//! It has minimal dependencies (serde, uuid) and is WASM-compatible, making it
//! suitable for use in both the kernel runtime and user-space capsule SDKs.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod ipc;
pub mod kernel;
pub mod llm;

pub use ipc::{IpcMessage, IpcPayload, OnboardingField, OnboardingFieldType, SelectionOption};
pub use kernel::{
    CapsuleMetadataEntry, CommandInfo, DaemonStatus, KernelRequest, KernelResponse,
    LlmProviderInfo, SYSTEM_SESSION_UUID,
};
pub use llm::{
    ContentPart, LlmResponse, LlmToolDefinition, Message, MessageContent, MessageRole, StopReason,
    StreamEvent, ToolCall, ToolCallResult, Usage,
};
