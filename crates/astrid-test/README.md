# astrid-test

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Shared test utilities, mocks, and fixtures for the Astrid workspace.

`astrid-test` is an internal `dev-dependency` used across the Astrid workspace. It provides a small but consistent set of building blocks: pre-built fixtures for core domain types (`AgentId`, `SessionId`, `ApprovalRequest`, `ElicitationRequest`), a `MockEventBus` for capturing emitted events, and a `TestContext` harness that manages temporary directories and tracing setup. Every helper is designed to reduce test boilerplate without hiding what a test is actually asserting.

## Core Features

- **Pre-built fixtures**: Instantly construct realistic instances of `AgentId`, `SessionId`, `ApprovalRequest`, and `ElicitationRequest` with sensible defaults or parameterized values.
- **Risk-level variants**: Dedicated helpers for medium-risk and high-risk approval fixtures, covering the common security test cases out of the box.
- **Elicitation schema helpers**: Factory functions for all three `ElicitationSchema` variants - `Text`, `Secret`, and `Confirm`.
- **Event capturing**: `MockEventBus` records emitted events in memory and exposes typed queries (`has_event`, `get_events_of_type`) for assertions without requiring a real event bus.
- **Temporary filesystem harness**: `TestContext` owns a `TempDir` that is cleaned up on drop, plus helpers to create files and subdirectories within it.
- **Tracing setup**: `setup_test_logging` and `setup_test_logging_default` initialize a `tracing-subscriber` scoped to the test writer, safe to call multiple times.

## Quick Start

Add as a `dev-dependency` inside the workspace:

```toml
[dev-dependencies]
astrid-test = { workspace = true }
```

Then import the prelude in your test module:

```rust,ignore
#[cfg(test)]
mod tests {
    use astrid_test::prelude::*;

    #[test]
    fn test_approval_fixture() {
        let req = test_approval_request();
        assert_eq!(req.operation, "test_operation");
    }
}
```

## API Reference

### Key Types

#### `TestContext`

A test context that owns a temporary directory for the lifetime of the test. Dropped at end of scope, which deletes the directory.

```rust,ignore
let ctx = TestContext::new();

// Create a file inside the temp dir
let path = ctx.create_file("config.toml", "[section]\nkey = \"value\"");
assert!(path.exists());

// Create a subdirectory
let dir = ctx.create_subdir("plugins");
assert!(dir.is_dir());

// Access the root path directly
println!("{}", ctx.path().display());
```

#### `MockEventBus`

An in-memory event bus that records every emitted event. `Clone`-able and `Arc`-backed, so it can be shared across threads or async tasks.

```rust,ignore
let bus = MockEventBus::new();

bus.emit("agent.started", serde_json::json!({ "agent_id": "abc" }));
bus.emit("tool.called",   serde_json::json!({ "tool": "read_file" }));

assert!(bus.has_event("agent.started"));

let tool_events = bus.get_events_of_type("tool.called");
assert_eq!(tool_events.len(), 1);

bus.clear();
assert!(!bus.has_event("agent.started"));
```

#### Fixture Functions

| Function | Returns | Notes |
|---|---|---|
| `test_agent_id()` | `AgentId` | Fresh random ID each call |
| `test_agent_id_from(uuid)` | `AgentId` | Deterministic from a known UUID |
| `test_session_id()` | `SessionId` | Fresh random ID each call |
| `test_session_id_from(uuid)` | `SessionId` | Deterministic from a known UUID |
| `test_approval_request()` | `ApprovalRequest` | `"test_operation"`, `RiskLevel::Medium` |
| `test_approval_request_for(op, desc)` | `ApprovalRequest` | Parameterized operation and description |
| `test_high_risk_approval()` | `ApprovalRequest` | `"delete_files"`, `RiskLevel::High`, resource set |
| `test_elicitation_request()` | `ElicitationRequest` | Default elicitation, no schema |
| `test_text_elicitation(msg)` | `ElicitationRequest` | `ElicitationSchema::Text`, max 1000 chars |
| `test_secret_elicitation(msg)` | `ElicitationRequest` | `ElicitationSchema::Secret` |
| `test_confirm_elicitation(msg)` | `ElicitationRequest` | `ElicitationSchema::Confirm`, default `false` |

#### Harness Functions

| Function | Returns | Notes |
|---|---|---|
| `test_dir()` | `TempDir` | Auto-cleaned on drop |
| `test_dir_with_prefix(prefix)` | `TempDir` | Named prefix for easier debugging |
| `test_file(content)` | `NamedTempFile` | Auto-cleaned on drop |
| `test_file_with_extension(content, ext)` | `NamedTempFile` | Useful when code checks extensions |
| `test_file_in_dir(dir, name, content)` | `PathBuf` | Creates parent directories as needed |
| `setup_test_logging(filter)` | `()` | Initializes tracing, safe to call multiple times |
| `setup_test_logging_default()` | `()` | Equivalent to `setup_test_logging("warn")` |

## Development

```bash
cargo test -p astrid-test
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
