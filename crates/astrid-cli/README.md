# astrid-cli

Command-line frontend for the Astrid secure agent runtime.

## Overview

This crate provides the CLI interface for interacting with Astrid, including
interactive chat sessions with LLM providers, session management, MCP server
control, and audit log inspection.

## Commands

### chat

Start an interactive chat session with the agent runtime.

```bash
# Start a new chat session
astrid chat

# Resume an existing session
astrid chat --session <SESSION_ID>
```

### sessions

Manage chat sessions.

```bash
# List all sessions
astrid sessions list

# Show session details
astrid sessions show <SESSION_ID>

# Delete a session
astrid sessions delete <SESSION_ID>
```

### servers

Manage MCP servers.

```bash
# List configured servers
astrid servers list

# List running servers
astrid servers running

# Start a server
astrid servers start <SERVER_NAME>

# Stop a server
astrid servers stop <SERVER_NAME>

# List available tools from all servers
astrid servers tools
```

### audit

View and verify audit logs.

```bash
# List audit sessions
astrid audit list

# Show audit entries for a session
astrid audit show <SESSION_ID>

# Verify audit chain integrity
astrid audit verify [SESSION_ID]

# Show audit statistics
astrid audit stats
```

## Global Options

- `-v, --verbose` - Enable verbose/debug output

## License

This crate is licensed under the MIT license.
