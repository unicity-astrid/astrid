# Phase 6: Ecosystem Registry & Universal Migrator

This document outlines the architecture for how Astrid OS discovers, packages, and installs extensions. The goal is to provide a frictionless "App Store" experience using decentralized infrastructure (GitHub) while strictly maintaining the purity of the Microkernel.

---

## 1. The Purity of the Core (`.capsule` and `Capsule.toml`)

Astrid adheres strictly to the Microkernel philosophy: **The core OS must only speak one language.**

The core daemon (`astridd`) will only ever understand the `Capsule.toml` manifest and the `.capsule` archive format (a `.tar.gz` containing the manifest, optional WASM, and static files). 

The OS does not know what an `mcp.json` is. It does not know what a `gemini-extension.json` is. If an artifact does not contain a `Capsule.toml`, the OS rejects it.

All ecosystem bridging, legacy parsing, and format conversion happens purely in user-space, inside the Developer Toolchain (`astrid build`).

---

## 2. Unapologetic Wrapping (The Universal Migrator)

To support the massive existing ecosystems of AI tooling, the Astrid CLI acts as a **Universal Migrator**. We unapologetically wrap and convert external standards into secure, native Astrid Capsules.

When a developer runs `astrid build` in a directory, the CLI uses heuristic detection to determine the conversion strategy:

### A. Astrid Native (Rust/WASM)
*   **Input:** `Cargo.toml` with `[package.metadata.astrid]`
*   **Action:** Compiles Rust to `wasm32-wasip1`. Boots the WASM in an ephemeral Extism VM to extract JSON schemas via the `astrid_export_schemas` hook.
*   **Output:** A `.capsule` containing `component.wasm` and a synthesized `Capsule.toml`.

### B. Standard MCP (`mcp.json`)
*   **Input:** `mcp.json`
*   **Action:** Extracts the host command (e.g., `npx`) and maps it to the `host_process` capability. Strips hardcoded API keys from the `env` block and converts them into interactive `[env]` elicitation requests in `Capsule.toml`.
*   **Output:** A `.capsule` containing a `Capsule.toml`. (No WASM is generated; the OS relies on the Host Escape Hatch to run the server).

### C. Gemini Extensions (`gemini-extension.json`)
*   **Input:** `gemini-extension.json` and associated static files (`GEMINI.md`, `skills/`).
*   **Action:** 
    1. Extracts `mcpServers` and converts them to `host_process` capabilities and `[[mcp_server]]` blocks.
    2. Converts the `settings` array into interactive `[env]` elicitation requests.
    3. Scans for local Markdown context (`GEMINI.md`) and maps it to `[[context_file]]`.
    4. Scans for local `.md` skills and maps them to `[[skill]]`.
*   **Output:** A `.capsule` containing the synthesized `Capsule.toml` and all associated static Markdown files.

---

## 3. Decentralized Discovery (The GitHub Registry)

Astrid does not rely on a centralized, proprietary package registry (like `npm` or `crates.io`) to host binary artifacts. Instead, it leverages **GitHub** as the global, decentralized registry.

When a user runs `astrid capsule install <target>`, the CLI follows a strict resolution pipeline:

1.  **Local File:** If `<target>` is a local path (e.g., `./dist/my-tool.capsule`), it installs directly.
2.  **Namespace Alias (`@org/repo`):** If `<target>` uses the npm-style scope prefix (e.g., `@upstash/context7` or `@modelcontextprotocol/server-github`), it automatically resolves to `https://github.com/org/repo`. This leverages GitHub's global namespace to prevent collisions and typosquatting without requiring a centralized alias database.
3.  **GitHub URL (`https://github.com/org/repo`):** If a raw URL is provided, it is used directly.

**For any remote GitHub resolution (Steps 2 and 3):**
*   **Phase A (Release Check):** The CLI checks the repository's GitHub Releases. If a `.capsule` asset is attached to the latest release, it downloads and installs it instantly. (This is the fastest, preferred distribution method for developers).
*   **Phase B (JIT Compilation):** If no `.capsule` release exists, the CLI clones the repository's source code to a temporary directory. It scans the source for `Capsule.toml`, `mcp.json`, or `gemini-extension.json`. If found, it automatically executes the `astrid build` pipeline (Section 2) on the fly, generating the `.capsule` archive locally, and then installs it.

### The App Store Website
Because the registry is decentralized, the community website (e.g., `registry.astralis.ai`) is simply a static UI. It queries the GitHub API for repositories tagged with `astrid-capsule` or `gemini-extension`, parses their `Capsule.toml` or `mcp.json` directly from the `main` branch to display documentation, and provides the user with the install command: `astrid capsule install github.com/user/repo`.

---

## 4. Milestone Checklist

- [x] **Step 6.1: The `.capsule` Archiver:** Finalize the `astrid build` archiving logic using `tar` and `flate2` to produce the standardized distribution format.
- [x] **Step 6.2: Gemini Extension Converter:** Implement the `gemini-extension.json` parsing logic in the CLI builder, properly mapping MCP servers, `settings` (env vars), `GEMINI.md`, and skills into a synthesized `Capsule.toml`.
- [x] **Step 6.3: Extism Schema Extraction:** Implement the local Extism VM boot sequence inside `astrid build` to extract `schemars` JSON from compiled Rust WASM binaries.
- [x] **Step 6.4: GitHub Resolution Pipeline:** Update `astrid capsule install` to support remote GitHub URLs, implementing both the Release download path and the JIT clone-and-build fallback.