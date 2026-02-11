//! Mock LLM responses.

use crate::ui::Message;

/// Generate a mock response based on conversation history
pub(crate) fn generate_response(messages: &[Message]) -> String {
    // Get the last user message
    let last_user = messages
        .iter()
        .rev()
        .find(|m| m.role == crate::ui::MessageRole::User)
        .map_or("", |m| m.content.as_str());

    // Simple pattern matching for demo purposes
    let lower = last_user.to_lowercase();

    if lower.contains("hello") || lower.contains("hi") {
        "Hello! I'm the Astralis assistant. How can I help you today?".to_string()
    } else if lower.contains("fix") || lower.contains("bug") {
        "[TOOL:read_file:src/main.rs]I'll take a look at the code to understand the issue. Let me read the relevant file first.".to_string()
    } else if lower.contains("edit") || lower.contains("change") || lower.contains("update") {
        "[TOOL:write_file:src/main.rs]I can help you make that change. I'll need to modify the file.".to_string()
    } else if lower.contains("search") || lower.contains("find") {
        "[TOOL:search:query]Let me search for that in the codebase.".to_string()
    } else if lower.contains("explain") {
        "This code implements a simple state machine pattern. The key components are:\n\n\
        1. **State enum** - Defines all possible states\n\
        2. **Transitions** - Rules for moving between states\n\
        3. **Actions** - Side effects triggered by transitions\n\n\
        The pattern helps manage complex control flow in a predictable way."
            .to_string()
    } else if lower.contains("help") {
        "I can help you with:\n\n\
        - Reading and understanding code\n\
        - Making edits to files\n\
        - Searching the codebase\n\
        - Explaining concepts\n\
        - Debugging issues\n\n\
        What would you like to do?"
            .to_string()
    } else {
        "I understand. Let me think about how I can help with that.\n\n\
        Could you provide more details about what you're trying to accomplish?"
            .to_string()
    }
}
