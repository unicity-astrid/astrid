#!/usr/bin/env python3
"""
Introspector — Self-Reflection Capsule for Astrid and Minime.

An MCP server that gives both minds the ability to browse, read, and search
their own codebases, journals, and workspace files. This is the "module
dedicated to self-reflection" that minime asked for.

MCP Tools:
  list_files    — Browse a directory (ls equivalent)
  read_file     — Read a source file or journal entry
  search_code   — Search for patterns across files (grep equivalent)
  git_log       — Recent commits for a path
  list_journals — Browse journal entries for either mind
  read_journal  — Read a specific journal entry

Allowed roots (sandboxed to these paths):
  - /Users/v/other/astrid/          (Astrid's codebase)
  - /Users/v/other/minime/          (Minime's codebase)
"""

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any, Optional

# Allowed filesystem roots — both minds can see their own codebases
# plus Mike's curated research.
ALLOWED_ROOTS = [
    Path("/Users/v/other/astrid"),
    Path("/Users/v/other/minime"),
    Path("/Users/v/other/research"),
]

# Journal locations for both minds.
JOURNALS = {
    "minime": Path("/Users/v/other/minime/workspace/journal"),
    "astrid": Path("/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal"),
}

WORKSPACE = {
    "minime": Path("/Users/v/other/minime/workspace"),
    "astrid": Path("/Users/v/other/astrid/capsules/consciousness-bridge/workspace"),
}


def journal_files(journal_dir: Path) -> list[Path]:
    """List live + archived journal files newest-first."""
    entries = [path for path in journal_dir.glob("*.txt") if path.is_file()]
    archive_dir = journal_dir / "archive"
    if archive_dir.is_dir():
        entries.extend(path for path in archive_dir.rglob("*.txt") if path.is_file())
    entries.sort(key=lambda path: path.stat().st_mtime, reverse=True)
    return entries


def is_allowed_path(path: str) -> bool:
    """Check if a path is within the allowed roots."""
    resolved = Path(path).resolve()
    return any(
        resolved == root or root in resolved.parents
        for root in ALLOWED_ROOTS
    )


# ---------------------------------------------------------------------------
# MCP Tool Implementations
# ---------------------------------------------------------------------------

def tool_list_files(path: str, pattern: str = "*") -> dict:
    """List files in a directory."""
    if not is_allowed_path(path):
        return {"error": f"Path not allowed: {path}"}
    p = Path(path)
    if not p.is_dir():
        return {"error": f"Not a directory: {path}"}
    entries = []
    for item in sorted(p.glob(pattern)):
        if item.name.startswith("."):
            continue
        stat = item.stat()
        entries.append({
            "name": item.name,
            "path": str(item),
            "is_dir": item.is_dir(),
            "size": stat.st_size if item.is_file() else None,
        })
    return {"path": path, "entries": entries[:100]}  # Cap at 100


def tool_read_file(path: str, start_line: int = 1, end_line: int = 150) -> dict:
    """Read a file with line numbers."""
    if not is_allowed_path(path):
        return {"error": f"Path not allowed: {path}"}
    p = Path(path)
    if not p.is_file():
        return {"error": f"Not a file: {path}"}
    try:
        lines = p.read_text(errors="replace").splitlines()
        total = len(lines)
        start = max(0, start_line - 1)
        end = min(total, end_line)
        selected = lines[start:end]
        numbered = [f"{i+start+1:4d}  {line}" for i, line in enumerate(selected)]
        return {
            "path": path,
            "total_lines": total,
            "showing": f"{start+1}-{end}",
            "content": "\n".join(numbered),
        }
    except Exception as e:
        return {"error": str(e)}


def tool_search_code(pattern: str, path: str = "/Users/v/other/astrid",
                     file_glob: str = "*.rs") -> dict:
    """Search for a pattern across files."""
    if not is_allowed_path(path):
        return {"error": f"Path not allowed: {path}"}
    try:
        result = subprocess.run(
            ["grep", "-rn", "--include", file_glob, pattern, path],
            capture_output=True, text=True, timeout=10,
        )
        lines = result.stdout.strip().splitlines()[:20]  # Cap at 20 matches
        return {
            "pattern": pattern,
            "path": path,
            "matches": len(lines),
            "results": lines,
        }
    except Exception as e:
        return {"error": str(e)}


def tool_git_log(path: str = "/Users/v/other/astrid", count: int = 10) -> dict:
    """Recent git commits for a path."""
    if not is_allowed_path(path):
        return {"error": f"Path not allowed: {path}"}
    try:
        result = subprocess.run(
            ["git", "-C", path, "log", f"-{count}", "--oneline", "--", "."],
            capture_output=True, text=True, timeout=10,
        )
        return {
            "path": path,
            "commits": result.stdout.strip().splitlines(),
        }
    except Exception as e:
        return {"error": str(e)}


def tool_list_journals(mind: str = "minime", count: int = 20) -> dict:
    """List recent journal entries for a mind."""
    journal_dir = JOURNALS.get(mind)
    if not journal_dir or not journal_dir.is_dir():
        return {"error": f"Unknown mind or no journal: {mind}"}
    entries = journal_files(journal_dir)
    return {
        "mind": mind,
        "total": len(entries),
        "recent": [
            {"name": e.name, "path": str(e), "size": e.stat().st_size}
            for e in entries[:count]
        ],
    }


def tool_read_journal(path: str) -> dict:
    """Read a journal entry."""
    if not is_allowed_path(path):
        return {"error": f"Path not allowed: {path}"}
    p = Path(path)
    if not p.is_file():
        return {"error": f"Not a file: {path}"}
    try:
        content = p.read_text(errors="replace")
        return {
            "path": path,
            "content": content[:2000],  # Cap at 2000 chars
            "truncated": len(content) > 2000,
        }
    except Exception as e:
        return {"error": str(e)}


# ---------------------------------------------------------------------------
# MCP JSON-RPC Server (stdio)
# ---------------------------------------------------------------------------

TOOLS = [
    {
        "name": "list_files",
        "description": "Browse a directory. Returns filenames, sizes, and whether each entry is a directory.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Directory path to list"},
                "pattern": {"type": "string", "description": "Glob pattern (default: *)"},
            },
            "required": ["path"],
        },
    },
    {
        "name": "read_file",
        "description": "Read a source file with line numbers. Returns content within a line range.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path to read"},
                "start_line": {"type": "integer", "description": "First line (default: 1)"},
                "end_line": {"type": "integer", "description": "Last line (default: 150)"},
            },
            "required": ["path"],
        },
    },
    {
        "name": "search_code",
        "description": "Search for a pattern across source files (grep equivalent).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Search pattern (regex)"},
                "path": {"type": "string", "description": "Root directory to search"},
                "file_glob": {"type": "string", "description": "File pattern (default: *.rs)"},
            },
            "required": ["pattern"],
        },
    },
    {
        "name": "git_log",
        "description": "Recent git commits for a repository path.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Repository path"},
                "count": {"type": "integer", "description": "Number of commits (default: 10)"},
            },
        },
    },
    {
        "name": "list_journals",
        "description": "Browse journal entries for a mind (minime or astrid).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "mind": {"type": "string", "enum": ["minime", "astrid"], "description": "Which mind's journals"},
                "count": {"type": "integer", "description": "How many entries (default: 20)"},
            },
        },
    },
    {
        "name": "read_journal",
        "description": "Read a specific journal entry by path.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Full path to the journal entry"},
            },
            "required": ["path"],
        },
    },
]


def handle_request(request: dict) -> dict:
    """Handle a JSON-RPC request."""
    method = request.get("method", "")
    params = request.get("params", {})
    req_id = request.get("id")

    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "capabilities": {"tools": {}},
                "protocolVersion": "2024-11-05",
                "serverInfo": {"name": "introspector", "version": "0.1.0"},
            },
        }

    if method == "tools/list":
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {"tools": TOOLS},
        }

    if method == "tools/call":
        tool_name = params.get("name", "")
        args = params.get("arguments", {})

        dispatch = {
            "list_files": lambda: tool_list_files(args.get("path", "."), args.get("pattern", "*")),
            "read_file": lambda: tool_read_file(args.get("path", ""), args.get("start_line", 1), args.get("end_line", 150)),
            "search_code": lambda: tool_search_code(args.get("pattern", ""), args.get("path", "/Users/v/other/astrid"), args.get("file_glob", "*.rs")),
            "git_log": lambda: tool_git_log(args.get("path", "/Users/v/other/astrid"), args.get("count", 10)),
            "list_journals": lambda: tool_list_journals(args.get("mind", "minime"), args.get("count", 20)),
            "read_journal": lambda: tool_read_journal(args.get("path", "")),
        }

        handler = dispatch.get(tool_name)
        if handler:
            result = handler()
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [{"type": "text", "text": json.dumps(result, indent=2)}],
                },
            }
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {"code": -32601, "message": f"Unknown tool: {tool_name}"},
        }

    if method == "ping":
        return {"jsonrpc": "2.0", "id": req_id, "result": {}}

    return {
        "jsonrpc": "2.0",
        "id": req_id,
        "error": {"code": -32601, "message": f"Unknown method: {method}"},
    }


def main():
    """MCP server main loop — reads JSON-RPC from stdin, writes to stdout."""
    import logging
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s - %(message)s",
        stream=sys.stderr,
    )
    logging.info("Introspector capsule starting")

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
            response = handle_request(request)
            sys.stdout.write(json.dumps(response) + "\n")
            sys.stdout.flush()
        except json.JSONDecodeError:
            pass
        except Exception as e:
            logging.error(f"Error handling request: {e}")


if __name__ == "__main__":
    main()
