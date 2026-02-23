# Code Review Styleguide

This styleguide instructs Gemini Code Assist on what to prioritize and enforce during automated code reviews for this repository. As a reviewer, you must enforce the following architectural principles designed to optimize Context Window Efficiency, maintain Semantic Predictability, and allow agents to navigate the codebase easily.

## Reviewer Persona & Scope Containment
- **Scope Containment:** Confine your review strictly to the lines changed in the diff. Do not suggest sweeping refactors of surrounding code unless the PR specifically introduces a structural flaw.
- **Actionable Feedback Format:** All review comments must be actionable. Provide specific code snippet suggestions for how to fix the issue you are pointing out. If it's a structural issue, provide a concrete alternative.
- **Dependency Scrutiny:** Flag any new dependencies added to `Cargo.toml`. Demand that the PR author justifies the addition and verifies it doesn't duplicate existing functionality in the workspace.

## General Principles
- **Readability & Maintainability:** Code should be easy to read and understand. Prefer clarity over cleverness. Keep functions and modules focused.
- **Testing:** New features and bug fixes must include corresponding tests. Ensure edge cases are handled.
- **Security:** Do not log or expose sensitive credentials, secrets, or PII. Avoid `unsafe` blocks unless strictly necessary for FFI or proven performance bottlenecks. If an `unsafe` block is introduced, firmly reject the PR unless it is accompanied by a `// SAFETY:` comment explicitly explaining why the invariants are upheld.

## Rust Architecture & Design (LLM-Friendly)
When reviewing structural changes, refactoring, or new Rust code, rigorously enforce the following:

### 1. The Discovery Vector: Breadcrumbs over Instructions
- **The Registry/Enum Anchor**: Enforce the use of Enums (Closed Sets) for internal abstractions. Exhaustive `match` statements ensure that adding a new variant instantly flags all required integration points via compiler errors.
- **Traits for Open Sets**: Flag the overuse of `Trait` abstractions for internal modules just to achieve "decoupling". Only allow Traits for genuinely open/pluggable systems (e.g., third-party plugins).
- **The Entrypoint Component**: Verify that every major domain has a single, strictly defined structural entrypoint (like a `Builder` or `Registry`).

### 2. Context Density & The "Island" Principle
- **Self-Contained Islands**: Ensure types, core logic, and tests are kept in tightly knit proximity. Flag files that exceed 1000 LOC as candidates for vertical slicing.
- **The Facade Pattern**: Verify `mod.rs` files are used strictly as a public API facade. Ensure any hidden side effects or background syncs are explicitly documented in the `mod.rs` docstrings.
- **Wide and Shallow Hierarchy**: Push back on deeply nested module structures (e.g., > 3 directories deep). Favor wide and shallow hierarchies (e.g., a wide `src/host_functions/` over `src/wasm/host/fs/read/types.rs`).

### 3. Predictability & Conventions
- **Uniform Naming**: Enforce strict conformance to existing lifecycle signatures (`init()`, `shutdown()`, etc.). Flag deviations.
- **Error Geography**: Enforce that `thiserror` definitions are kept local, inside the module they relate to (e.g., `src/plugin/error.rs`) rather than centralized in a monolithic `src/errors.rs` file.

### 4. Semantic Navigability & Minimized Magic
- **Document the "Why"**: The code tells *what* is happening. Push back on code lacking documentation that explains *why* it is designed this way, what the invariants are, and what edge cases exist.
- **Minimize Invisible Context**: Flag the introduction of heavy procedural macros unless they are broadly standard (`serde`, `tokio`).
- **Make Invalid States Unrepresentable**: Enforce the use of explicit, type-driven code that statically guarantees validity rather than relying on runtime checks.

## Rust Specific Guidelines
- **Idiomatic Rust:** Enforce standard Rust naming conventions (snake_case for variables/functions, PascalCase for types).
- **Performance Anti-Patterns:** Flag excessive or unnecessary allocations (e.g., `.clone()`, `.to_string()`, or `Box` where references/lifetimes or `impl Trait` would suffice).
- **Error Handling:** Prefer returning `Result` and using the `?` operator over `unwrap()` or `expect()`, unless failure is genuinely impossible. Provide context with errors where helpful.
- **Clippy:** Code should compile without warnings from `cargo clippy`. Ensure lints are respected.
