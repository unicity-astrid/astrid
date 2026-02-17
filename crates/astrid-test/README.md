# astrid-test

Shared test utilities for the Astrid secure agent runtime.

## Overview

This crate provides mock implementations and test helpers for testing Astrid crates. It includes:

- **MockFrontend** - Mock implementation of the `Frontend` trait
- **MockEventBus** - Event capturing for integration tests
- **Test fixtures** - Pre-built approval requests, elicitation requests, IDs
- **Test harness** - Temp directories, files, logging setup, `TestContext`

## Installation

Add as a dev-dependency in your crate's `Cargo.toml`:

```toml
[dev-dependencies]
astrid-test.workspace = true
```

## Usage

```rust
#[cfg(test)]
mod tests {
    use astrid_test::{MockFrontend, test_approval_request};
    use astrid_core::ApprovalOption;

    #[tokio::test]
    async fn test_approval_flow() {
        let frontend = MockFrontend::new();
        frontend.queue_approval(ApprovalOption::AllowOnce).await;

        let request = test_approval_request();
        let decision = frontend.request_approval(request).await.unwrap();

        assert!(decision.is_approved());
    }
}
```

### Test Harness

```rust
use astrid_test::{TestContext, setup_test_logging};

#[test]
fn test_with_temp_files() {
    setup_test_logging("debug");

    let ctx = TestContext::new();
    let file_path = ctx.create_file("config.toml", "[settings]");

    assert!(file_path.exists());
}
```

## License

This crate is licensed under the MIT license.
