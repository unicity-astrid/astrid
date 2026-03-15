# astrid-cli-mockup

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The TUI design prototype for the Astrid OS.**

A self-contained ratatui mockup with zero runtime dependencies. No LLM calls, no database, no daemon. Every agent action, tool execution, and approval flow is simulated with scripted demo scenarios. This is where the dashboard UX gets designed and validated before it touches real infrastructure.

## Why it exists

The real CLI (`astrid-cli`) connects to a live kernel daemon. Designing UX against a live system is slow and fragile. This mockup decouples visual design from runtime correctness. It runs standalone, produces deterministic output in snapshot mode, and iterates at the speed of `cargo run`.

## Views

Nine views across four sections, navigable by number key or Tab:

| Section | Views |
|---|---|
| **Operate** | Nexus (1), Missions (2), Atlas (3) |
| **Control** | Command (4), Topology (5), Shield (6) |
| **Monitor** | Chain (7), Pulse (8) |
| **Utility** | Console (0) |

## Demo system

Ten scripted scenarios auto-play through the state machine. Each scenario drives typed input, agent streaming, tool requests, approval prompts, and view switches with configurable timing.

Scenarios: `simple-qa`, `file-read`, `file-write`, `approval-flow`, `error-recovery`, `multi-tool`, `multi-agent-ops`, `quick`, `showcase`, `full-demo`.

## Quick start

```bash
# Interactive mode
cargo run -p astrid-cli-mockup

# Auto-playing demo
cargo run -p astrid-cli-mockup -- --demo showcase

# Non-interactive snapshot (prints rendered frames to stdout)
cargo run -p astrid-cli-mockup -- --snapshot showcase --steps 5
```

## Development

```bash
cargo test -p astrid-cli-mockup
```

This crate has `publish = false` and is not released to crates.io.

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
