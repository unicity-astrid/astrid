//! Mock tool call extraction and simulation.

use crate::ui::state::RiskLevel;

/// A mock tool call extracted from a response
pub(crate) struct MockToolCall {
    pub name: String,
    pub description: String,
    pub risk: RiskLevel,
    pub details: Vec<(String, String)>,
}

/// Extract a tool call from a response (if present)
pub(crate) fn extract_tool_call(response: &str) -> Option<MockToolCall> {
    // Look for [TOOL:name:arg] pattern
    if let Some(start) = response.find("[TOOL:") {
        // Safety: start is from find(), so start + 6 is within bounds of the "[TOOL:" match
        #[allow(clippy::arithmetic_side_effects)]
        let rest = &response[start + 6..];
        if let Some(end) = rest.find(']') {
            let tool_spec = &rest[..end];
            let parts: Vec<&str> = tool_spec.split(':').collect();

            if parts.len() >= 2 {
                let name = parts[0].to_string();
                let arg = parts[1].to_string();

                let (risk, description) = match name.as_str() {
                    "read_file" => (RiskLevel::Low, format!("Read file: {arg}")),
                    "write_file" => (RiskLevel::Medium, format!("Write to file: {arg}")),
                    "delete_file" => (RiskLevel::High, format!("Delete file: {arg}")),
                    "search" => (RiskLevel::Low, format!("Search codebase for: {arg}")),
                    "execute" => (RiskLevel::High, format!("Execute command: {arg}")),
                    _ => (RiskLevel::Medium, format!("Tool: {name} with {arg}")),
                };

                return Some(MockToolCall {
                    name,
                    description,
                    risk,
                    details: vec![
                        ("Path".to_string(), arg),
                        ("Workspace".to_string(), "~/projects/demo".to_string()),
                    ],
                });
            }
        }
    }

    None
}
