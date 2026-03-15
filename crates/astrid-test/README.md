# astrid-test

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The shared test utility crate for the Astrid OS.**

An internal `dev-dependency` used across the workspace. Provides pre-built fixtures for core domain types, a `MockEventBus` for capturing emitted events in memory, and a `TestContext` harness that manages temporary directories and tracing setup. Reduces boilerplate without hiding what a test actually asserts.

## Why it exists

Every crate in the workspace needs `ApprovalRequest` fixtures, temp directories, and tracing initialization. Without a shared test crate, each crate duplicates this setup, and the duplicates drift. `astrid-test` is the single source for test infrastructure.

## What it provides

**Fixtures.** Factory functions that construct realistic domain objects with sensible defaults. `test_approval_request()` returns a medium-risk approval. `test_high_risk_approval()` returns a high-risk approval targeting `/home/user/important`. `test_text_elicitation(msg)`, `test_secret_elicitation(msg)`, and `test_confirm_elicitation(msg)` cover all three `ElicitationSchema` variants. `test_agent_id()` and `test_session_id()` produce unique IDs on every call.

**Mocks.** `MockEventBus` records emitted events in an `Arc<Mutex<Vec<MockEvent>>>`. Call `emit(type, payload)` to record, `has_event(type)` to assert, `get_events_of_type(type)` to filter. Clone-able and thread-safe for use in concurrent tests.

**Harness.** `TestContext` owns a `TempDir` cleaned up on drop, with helpers for `create_file(name, content)` and `create_subdir(name)`. Standalone functions `test_dir()`, `test_file(content)`, `test_file_with_extension(content, ext)`, and `test_file_in_dir(dir, name, content)` cover the common cases. `setup_test_logging(filter)` initializes `tracing-subscriber` scoped to the test writer, safe to call multiple times.

## Quick start

```toml
[dev-dependencies]
astrid-test = { workspace = true }
```

```rust
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

## Development

```bash
cargo test -p astrid-test
```

This crate has `publish = false` and is not released to crates.io.

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
