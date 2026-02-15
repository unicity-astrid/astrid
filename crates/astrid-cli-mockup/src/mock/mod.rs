//! Mock implementations for simulating LLM and tool responses.
//!
//! These are used for interactive mode (non-demo) where
//! user types and mock responses are generated.

#[allow(dead_code)]
mod responses;
#[allow(dead_code)]
mod tools;

#[allow(unused_imports)]
pub(crate) use responses::generate_response;
#[allow(unused_imports)]
pub(crate) use tools::{MockToolCall, extract_tool_call};
