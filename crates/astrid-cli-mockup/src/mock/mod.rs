//! Mock implementations for simulating LLM and tool responses.
//!
//! These are used for interactive mode (non-demo) where
//! user types and mock responses are generated.

#[expect(dead_code)]
mod responses;
#[expect(dead_code)]
mod tools;

#[expect(unused_imports)]
pub(crate) use responses::generate_response;
#[expect(unused_imports)]
pub(crate) use tools::{MockToolCall, extract_tool_call};
