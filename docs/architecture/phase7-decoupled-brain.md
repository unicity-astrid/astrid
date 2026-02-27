# Phase 7: The Decoupled Brain (LLMs as Capsules)

This document details the architectural migration required to fully realize Astrid as a pure Microkernel Operating System. 

The goal of Phase 7 is to completely eject the "Brain" (the LLM providers and the agent orchestration loop) from the core OS daemon (`astrid-runtime`) and move them entirely into User-Space Capsules.

---

## 1. The Core Problem: The Monolithic Brain
Currently, Astrid is a hybrid. It uses a secure WASM microkernel for *Tools* (Phase 4/5), but the actual "Intelligence" is hardcoded into the kernel:
*   `astrid-llm` is compiled directly into the daemon, hardcoding the Anthropic and OpenAI HTTP clients.
*   `astrid-runtime` sits in the daemon, running a hardcoded loop: `Send Prompt -> Stream Tokens -> Detect Tool Call -> Execute Tool -> Repeat`.

This means if a user wants to use a new LLM provider (like Groq or a local Ollama), or change the agentic reasoning loop (e.g., swapping a standard ReAct loop for a Monte Carlo Tree Search loop), they have to wait for a core OS update and recompile the daemon. 

This violates the Microkernel philosophy. The OS should be dumb; the intelligence should be swappable.

---

## 2. The Solution: Intelligence in User-Space

In Phase 7, the core OS daemon (`astridd`) is stripped down to merely managing the IPC Event Bus, the VFS, and the Sandbox. 

All intelligence is moved into specific Capsule roles:

### A. The Provider Capsules (e.g., `astrid-capsule-anthropic`)
These capsules are responsible solely for communicating with specific LLM APIs.
*   **Capabilities:** Require `net` access to their specific API endpoints (e.g., `api.anthropic.com`).
*   **Mechanism:** They listen on the Event Bus for `llm.request.generate` events. When received, they use the `astrid::sys::http_request` Airlock to call the API, parse the proprietary response (e.g., Claude's specific Server-Sent Events format), and publish standardized `llm.stream.token` events back to the Event Bus.

### B. The Orchestrator Capsule (`astrid-capsule-orchestrator`)
This capsule replaces `astrid-runtime`. It is the actual "Agent."
*   **Mechanism:** It listens to the frontend for `user.prompt` events. It maintains the conversation history in its own isolated state using the `astrid::sys::kv` Airlocks. It sends generation requests to the Provider Capsules, parses incoming tool calls, and sends execution requests to the Tool Router.

### C. The Tool Router Capsule
Because the Orchestrator doesn't know *which* capsule owns `run_shell_command` or `read_file`, this middleware capsule listens for `tool.request.execute` events from the Orchestrator, looks up the tool in the registry, and forwards the execution payload to the correct User-Space Tool Capsule (`astrid-capsule-fs`, `astrid-capsule-shell`, etc.).

---

## 3. The "Distro" Metaphor

By ejecting the brain, we create the concept of **Astrid Distributions (Distros)**.

Because the OS is just a dumb sandbox, community members can create entirely different "Distros" of Astrid simply by swapping out the default capsules in their `config.toml`:

*   **The "Standard" Distro:** Uses the Anthropic capsule and the standard ReAct orchestrator.
*   **The "Paranoid" Distro:** Uses a Local LLaMA provider capsule and strictly disables the `astrid-capsule-shell` tool.
*   **The "Swarm" Distro:** Replaces the standard orchestrator with a multi-agent swarm orchestrator capsule that breaks tasks down and coordinates multiple sub-agents simultaneously over the Event Bus.

The core OS code remains mathematically identical across all of them.

---

## 4. Implementation Steps

This is the "heart transplant" of the OS. It must be done atomically to avoid breaking the system.

- [ ] **Step 7.1: The Standardized LLM IPC Schema:** Define the exact byte payloads for `LlmRequest`, `LlmResponse`, `TokenDelta`, and `ToolCallDelta` in `astrid-events`. These must be universal across all models.
- [ ] **Step 7.2: Provider Extraction (`astrid-capsule-anthropic`):** Extract the `claude.rs` logic from `astrid-llm` into a pure WASM capsule using the HTTP Airlocks and the new IPC schemas.
- [ ] **Step 7.3: Orchestrator Extraction (`astrid-capsule-orchestrator`):** Extract the `astrid-runtime::execution` loop into a stateful WASM capsule that communicates purely over the Event Bus.
- [ ] **Step 7.4: The Front-End Re-Wire:** Update the existing CLI and Telegram frontends to stop calling `astrid-runtime` directly, and instead publish `user.prompt` events to the Event Bus and listen for `agent.response` events.
- [ ] **Step 7.5: The Great Purge:** Delete `astrid-runtime` and `astrid-llm` from the core daemon tree entirely.

## 5. Extensibility Use Cases

By enforcing the rule that all intelligence must communicate solely via standard IPC schemas over the Event Bus, Astrid becomes a truly infinite sandbox for AI research and deployment.

### Use Case 1: The Local "Paranoid" Distro
A financial firm requires absolute data privacy. They cannot use Anthropic or OpenAI. 
*   **The Swap:** They uninstall `astrid-capsule-anthropic` and install `astrid-capsule-ollama`.
*   **The Result:** The Orchestrator capsule still emits `llm.request.generate` events on the bus. The new Ollama capsule catches them and routes them to a local GPU. The Orchestrator doesn`t even know the provider changed; the entire OS functions offline instantly.

### Use Case 2: The "Debate Swarm" Researcher
A university researcher wants to test a novel agentic architecture where three distinct personas (A Coder, a Security Expert, and a Manager) debate a user`s prompt before executing a tool.
*   **The Swap:** The researcher writes a new `astrid-capsule-debate-orchestrator`. They update `config.toml` to route `user.prompt` events to their new capsule instead of the default ReAct orchestrator.
*   **The Result:** The new orchestrator receives the user`s prompt. It emits *three* parallel `llm.request` events to three different provider capsules. It aggregates their responses over the Event Bus, synthesizes a final plan, and sends the winning `tool.request.execute` to the Tool Router. 

### Use Case 3: The Caching Middleware
A company wants to reduce LLM API costs for repetitive CLI tasks (like asking "How do I undo a git commit?").
*   **The Swap:** They install a middleware `astrid-capsule-cache`. They configure the Event Bus router to send all `llm.request` events to the Cache capsule *first*. 
*   **The Result:** The Cache capsule intercepts the request. It uses the `astrid::sys::kv` airlocks to check its local database. If it finds a match, it instantly publishes the `LlmResponse` back to the Orchestrator. If it misses, it re-publishes the event to the actual Provider capsule. The Orchestrator and the Provider remain completely unmodified.

### Use Case 4: The "Self-Driving" Test Suite
A developer wants an agent to autonomously write code, run tests, and fix bugs in a loop overnight without human intervention.
*   **The Swap:** They replace the default Orchestrator with an `astrid-capsule-autonomous-worker`. 
*   **The Result:** This specialized orchestrator catches `user.prompt` ("Build a web server"), but instead of waiting for human approval after every tool call, it programmatically loops through the Tool Router, reads the `astrid-capsule-shell` exit codes, and self-corrects based on compiler errors until the tests pass.

