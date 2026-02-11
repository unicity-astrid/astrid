# astralis-cli

Command-line frontend for the Astralis secure agent runtime.

## Overview

This crate provides the CLI interface for interacting with Astralis, including
interactive chat sessions with LLM providers, session management, MCP server
control, and audit log inspection.

## Commands

### chat

Start an interactive chat session with the agent runtime.

```bash
# Start a new chat session
astralis chat

# Resume an existing session
astralis chat --session <SESSION_ID>
```

### sessions

Manage chat sessions.

```bash
# List all sessions
astralis sessions list

# Show session details
astralis sessions show <SESSION_ID>

# Delete a session
astralis sessions delete <SESSION_ID>
```

### servers

Manage MCP servers.

```bash
# List configured servers
astralis servers list

# List running servers
astralis servers running

# Start a server
astralis servers start <SERVER_NAME>

# Stop a server
astralis servers stop <SERVER_NAME>

# List available tools from all servers
astralis servers tools
```

### audit

View and verify audit logs.

```bash
# List audit sessions
astralis audit list

# Show audit entries for a session
astralis audit show <SESSION_ID>

# Verify audit chain integrity
astralis audit verify [SESSION_ID]

# Show audit statistics
astralis audit stats
```

## Global Options

- `-v, --verbose` - Enable verbose/debug output

## License

This crate is licensed under the MIT license.
