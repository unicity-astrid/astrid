# astrid-workspace

Operational workspace boundaries for the Astrid secure agent runtime.

## Overview

This crate defines where an agent can freely operate and when it needs user
approval to "escape" those boundaries. Unlike the WASM sandbox (which is
inescapable for security), the operational workspace is a user-controlled
boundary that can be relaxed with approval.

## Key Concepts

- **Workspace**: A directory tree where the agent operates without restrictions
- **Escape**: Operations outside the workspace require user approval
- **Modes**: Control how strictly boundaries are enforced

## Workspace Modes

| Mode | Behavior |
|------|----------|
| **Safe** | Always ask for approval on any escape attempt |
| **Guided** | Smart defaults - ask for risky operations, allow safe ones |
| **Autonomous** | No restrictions (for agent-only machines) |

## Usage

```rust
use astrid_workspace::{WorkspaceBoundary, WorkspaceConfig, WorkspaceMode};

let config = WorkspaceConfig::new("/home/user/project")
    .with_mode(WorkspaceMode::Guided);

let boundary = WorkspaceBoundary::new(config);

// Check if a path is allowed
match boundary.check("/home/user/project/src/main.rs") {
    PathCheck::Allowed => println!("Path is in workspace"),
    PathCheck::RequiresApproval => println!("Needs user approval"),
    _ => {}
}
```

## Key Types

- `WorkspaceBoundary` - Evaluates paths against workspace rules
- `WorkspaceConfig` - Configuration for workspace boundaries
- `WorkspaceMode` - Safe, Guided, or Autonomous operation
- `PathCheck` - Result of checking a path (Allowed, RequiresApproval, Denied)
- `EscapeRequest` - Request to operate outside the workspace
- `EscapeDecision` - User's response to an escape request

## Escape vs WASM Sandbox

| Aspect | Workspace (this crate) | WASM Sandbox |
|--------|------------------------|--------------|
| Purpose | User control over agent actions | Code execution safety |
| Escapable | Yes, with approval | Never |
| Enforced by | Policy + capabilities | WASM runtime |
| User override | Yes (autonomous mode) | No |

## License

This crate is licensed under the MIT license.
