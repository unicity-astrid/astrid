# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-03-15

### Added

- RFC system for capsule interface standards (`astrid-rfcs` repo)
- RFC-0001: HTTP Fetch tool interface
- SECURITY.md vulnerability reporting policy
- MSRV enforcement in CI (Rust 1.94)
- `cargo-audit` security audit in CI
- Tag-triggered release pipeline with GitHub Releases
- `serde`, `serde_json`, `borsh` re-exports from `astrid-sdk`

### Changed

- Extracted all capsules into standalone polyrepo repositories
- Standardized all Cargo.toml files (alpha-sorted deps, workspace inheritance, authors)
- Bumped all workspace crates to 0.2.0

### Removed

- Capsule crates from core workspace (now standalone repos)
- Test guest plugin crate

## [0.1.1] - 2026-03-10

### Added

- Multi-line paste block support in TUI input
- Background process management (spawn, logs, kill)
- IPC blocking receive with timeout
- Capsule interceptor auto-subscribe system
- Identity resolution and platform linking
- Human approval gates for sensitive actions
- Schema-aware onboarding with elicitation prompts
- OpenClaw JS/TS capsule compilation pipeline

### Fixed

- Daemon handshake race and orphan processes
- VFS atomic clear_prefix

### Security

- Defense-in-depth sandbox hardening
- Capability token validation improvements
- Audit chain integrity checks

[Unreleased]: https://github.com/unicity-astrid/astrid/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/unicity-astrid/astrid/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/unicity-astrid/astrid/releases/tag/v0.1.1
