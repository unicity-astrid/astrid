# astrid-llm

[![Crates.io](https://img.shields.io/crates/v/astrid-llm)](https://crates.io/crates/astrid-llm)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

A unified, streaming-first abstraction layer for Large Language Models within the Astralis OS.

`astrid-llm` provides a standard interface (`LlmProvider`) across disparate AI models, enabling the core agentic runtime of Astralis to interact seamlessly with Anthropic Claude, OpenAI-compatible endpoints (including local models via LM Studio or vLLM), and Z.AI. It handles the complexities of server-sent events (SSE) parsing, tool calling schemas, and provider-specific nuances so the rest of the OS doesn't have to.

## Core Features

- **Unified Message Format**: A standardized `Message` and `MessageContent` system that translates Astralis types into provider-specific API formats (e.g., Anthropic's block format vs. OpenAI's content arrays).
- **First-Class Streaming**: Returns a `StreamBox` of `StreamEvent` items, allowing the OS to react to text deltas, tool call chunks, and reasoning tokens in real-time.
- **Native Tool Calling**: Standardized schemas for passing tool definitions to models and robust parsing for translating model outputs back into actionable `ToolCall` and `ToolCallResult` structures.
- **Reasoning Tokens**: Built-in support for chain-of-thought streaming (e.g., `<think>` blocks in DeepSeek or Z.AI models) via the `ReasoningDelta` event.

## Architecture

This crate is designed around a single, highly flexible `LlmProvider` trait. By standardizing messages, tool definitions, and streaming events, Astralis can switch between frontier models and local, air-gapped models without changing any orchestration logic.

### Supported Providers

#### Claude (Anthropic)
Implementation for the Anthropic API (`ClaudeProvider`). Automatically handles Anthropic's unique tool-use block format, stop reasons, and version headers. Default configuration targets Claude 3.5 Sonnet.

#### OpenAI-Compatible
A highly versatile provider (`OpenAiCompatProvider`) designed for any endpoint implementing the OpenAI Chat Completions API.
- **Local Models**: Pre-configured initializers for LM Studio (`lm_studio()`).
- **Cloud Models**: Direct support for OpenAI APIs.
- **Custom Endpoints**: Configurable for vLLM, Ollama, or custom inferences servers via custom base URLs.

#### Z.AI (Zhipu AI)
Implementation for the GLM-4 series (`ZaiProvider`). Handles Z.AI's specific extensions to the OpenAI format, including `reasoning_content` deltas and custom stop reasons like `sensitive` and `network_error`.

## Quick Start

### Basic Completion (Claude)

```rust
use astrid_llm::{ClaudeProvider, LlmProvider, Message, ProviderConfig};

async fn example() -> Result<(), astrid_llm::LlmError> {
    let config = ProviderConfig::new("your-api-key", "claude-3-5-sonnet-20241022");
    let provider = ClaudeProvider::new(config);

    let response = provider.complete_simple("Explain the Astralis architecture.").await?;
    println!("Response: {}", response);
    
    Ok(())
}
```

### Local Model Streaming (LM Studio)

```rust
use astrid_llm::{OpenAiCompatProvider, LlmProvider, Message, StreamEvent};
use futures::StreamExt;

async fn example() -> Result<(), astrid_llm::LlmError> {
    // Connects to http://localhost:1234 by default
    let provider = OpenAiCompatProvider::lm_studio_with_model("llama-3.1-8b");
    let messages = vec![Message::user("Initialize system diagnostics.")];

    let mut stream = provider.stream(&messages, &[], "You are an Astralis OS kernel agent.").await?;

    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::TextDelta(text) => print!("{}", text),
            StreamEvent::Done => println!("\n[Stream Complete]"),
            _ => {}
        }
    }
    
    Ok(())
}
```

## API Reference

- **`Message`**: Represents a single turn in the conversation (`System`, `User`, `Assistant`, `Tool`).
- **`LlmToolDefinition`**: Defines a capability exposed to the LLM, requiring a name, description, and JSON schema.
- **`StreamEvent`**: An enum representing the lifecycle of a streaming request:
  - `TextDelta`: Standard text output.
  - `ReasoningDelta`: Chain-of-thought output.
  - `ToolCallStart` / `ToolCallDelta` / `ToolCallEnd`: The lifecycle of a function call.
  - `Usage`: Final token counts.
- **`LlmResponse`**: The final aggregate response for non-streaming requests, including the full message, stop reason, and token usage.

## Development

```bash
cargo test -p astrid-llm
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
