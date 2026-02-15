# astrid-llm

LLM provider abstraction with streaming support for Astrid.

## Features

- **Provider Trait** - Unified `LlmProvider` trait for LLM abstraction
- **Claude (Anthropic)** - Full support for Claude models
- **OpenAI-Compatible** - Works with LM Studio, OpenAI, vLLM, and other compatible APIs
- **Streaming** - Real-time streaming response support
- **Tool Use** - Built-in tool/function calling support

## Usage

### Claude (Anthropic)

```rust
use astrid_llm::{ClaudeProvider, LlmProvider, ProviderConfig};

// Create provider with API key
let config = ProviderConfig::new("your-api-key", "claude-sonnet-4-20250514");
let provider = ClaudeProvider::new(config);

// Or load from ANTHROPIC_API_KEY environment variable
let provider = ClaudeProvider::from_env()?;

// Simple completion
let response = provider.complete_simple("What is 2+2?").await?;
println!("Response: {}", response);
```

### LM Studio (Local)

```rust
use astrid_llm::{OpenAiCompatProvider, LlmProvider};

// Connect to LM Studio running locally (default: http://localhost:1234)
let provider = OpenAiCompatProvider::lm_studio();

// Or with a specific model
let provider = OpenAiCompatProvider::lm_studio_with_model("llama-3.1-8b");

let response = provider.complete_simple("Hello!").await?;
println!("Response: {}", response);
```

### Streaming

```rust
use astrid_llm::{ClaudeProvider, LlmProvider, Message, StreamEvent};
use futures::StreamExt;

let provider = ClaudeProvider::from_env()?;
let messages = vec![Message::user("Tell me a story")];

let mut stream = provider.stream(&messages, &[], "").await?;

while let Some(event) = stream.next().await {
    match event? {
        StreamEvent::TextDelta(text) => print!("{}", text),
        StreamEvent::Done => println!("\n[Done]"),
        _ => {}
    }
}
```

## Key Types

| Type | Description |
|------|-------------|
| `LlmProvider` | Core trait for LLM providers |
| `ClaudeProvider` | Anthropic Claude implementation |
| `OpenAiCompatProvider` | OpenAI-compatible API implementation |
| `Message` | Chat message (user/assistant/system) |
| `LlmResponse` | Complete response with content and usage |
| `StreamEvent` | Streaming event (text delta, tool calls, done) |
| `ProviderConfig` | Provider configuration (API key, model, etc.) |

## License

This crate is licensed under the MIT license.
