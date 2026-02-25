# astrid-test

[![Crates.io](https://img.shields.io/crates/v/astrid-test)](https://crates.io/crates/astrid-test)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Shared test utilities, mocks, and fixtures for the Astralis OS workspace. 

`astrid-test` provides a robust, deterministic testing environment for the Astralis secure agent runtime. Instead of relying on live LLM APIs or manual user interactions, this crate exposes predictable mock implementations of core interfaces like `LlmProvider` and `Frontend`. It ensures that agent loops, tool-call flows, and security policies can be thoroughly verified in CI environments without external dependencies or side effects.

## Core Features

* **Deterministic LLM Mocking**: Script and replay exact sequences of text responses, tool calls, or errors using `MockLlmProvider`.
* **Frontend Simulation**: Automate user approvals, inputs, and elicitations with the queue-based `MockFrontend`.
* **Event Capturing**: Intercept and assert on telemetry and system events via `MockEventBus`.
* **Pre-built Fixtures**: Instantly generate realistic dummy data for `AgentId`, `SessionId`, `ApprovalRequest`, and `ElicitationRequest`.
* **Test Harness Environment**: Safely manage temporary directories, files, and tracing loggers tailored for tests.

## Architecture

This crate is purely a `dev-dependency` mechanism for the rest of the Astralis workspace. It depends on core domain traits defined in `astrid-core` and `astrid-llm` and implements them using in-memory structures (`Arc<Mutex<VecDeque<T>>>`). This architecture allows test environments to instantiate an entire isolated Astralis runtime kernel where the environment, LLM, and user are completely synthesized.

## Quick Start

Because `astrid-test` is designed exclusively for testing, it should only be included as a `dev-dependency` within your crate.

```toml
[dev-dependencies]
astrid-test = { workspace = true }
```

### `MockLlmProvider`

Testing agent flows often requires strict control over what the language model generates. `MockLlmProvider` allows you to preload a queue of `MockLlmTurn`s. Each time the provider is invoked by the runtime, it pops the next turn, seamlessly simulating streaming text, multiple tool calls, or network errors.

```rust
use astrid_test::{MockLlmProvider, MockLlmTurn, MockToolCall};
use serde_json::json;

// Setup a scripted sequence of LLM responses
let provider = MockLlmProvider::new(vec![
    // First turn: The LLM decides to call a tool
    MockLlmTurn::tool_calls(vec![
        MockToolCall::new("read_file", json!({ "path": "/etc/config" }))
    ]),
    // Second turn: The LLM provides a text summary
    MockLlmTurn::text("The configuration looks valid."),
]);

// The provider can now be injected into an agent context.
// You can also inspect captured messages after execution:
let history = provider.captured_messages();
assert_eq!(provider.call_count(), 2);
```

### `MockFrontend`

The `Frontend` trait governs how Astralis requests user permission or input. `MockFrontend` uses a thread-safe queue system (allowing both synchronous and asynchronous usage) to pre-approve operations or provide simulated user text.

```rust
use astrid_test::{MockFrontend, test_approval_request};
use astrid_core::ApprovalOption;

#[tokio::test]
async fn test_approval_flow() {
    // Configure the mock to automatically allow the next request
    let frontend = MockFrontend::new()
        .with_approval_response(ApprovalOption::AllowOnce);

    // Generate a standardized dummy request
    let request = test_approval_request();
    
    // Execute the trait method
    let decision = frontend.request_approval(request).await.unwrap();

    assert!(decision.is_approved());
    assert_eq!(decision.decision, ApprovalOption::AllowOnce);
}
```

### Fixtures & Harnesses

Reduce boilerplate when setting up integration tests by utilizing the provided `TestContext` and fixture generators.

```rust
use astrid_test::{TestContext, test_high_risk_approval, setup_test_logging};

#[test]
fn test_fs_operations() {
    setup_test_logging("debug"); // Initializes tracing-subscriber
    
    // TestContext automatically manages temporary directories
    let ctx = TestContext::new();
    let file_path = ctx.create_file("dummy.txt", "Hello Astralis");
    
    assert!(file_path.exists());
    
    // Use pre-configured high-risk policies for security testing
    let high_risk_req = test_high_risk_approval();
    assert_eq!(high_risk_req.operation, "delete_files");
}
```

## Development

```bash
cargo test -p astrid-test
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
