#!/usr/bin/env python3
"""Resolve an Astrid agency request and notify her through the inbox."""

from __future__ import annotations

import argparse
import json
import time
from pathlib import Path


DEFAULT_INBOX = Path("/Users/v/other/astrid/capsules/consciousness-bridge/workspace/inbox")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Update an agency request status and write an Astrid inbox note."
    )
    parser.add_argument("--request", required=True, help="Path to the request JSON file")
    parser.add_argument(
        "--status",
        required=True,
        choices=["accepted", "completed", "declined"],
        help="New status for the request",
    )
    parser.add_argument(
        "--summary",
        required=True,
        help="Concrete outcome summary that Astrid will read back",
    )
    parser.add_argument(
        "--file",
        action="append",
        default=[],
        help="Touched file path. Repeat for multiple files.",
    )
    parser.add_argument(
        "--artifact",
        help="Optional artifact path or description to report back",
    )
    parser.add_argument(
        "--inbox-dir",
        default=str(DEFAULT_INBOX),
        help="Override inbox directory for testing or alternate setups",
    )
    return parser.parse_args()


def build_note(request: dict, status: str, summary: str, files: list[str], artifact: str | None) -> str:
    lines = [
        "=== AGENCY REQUEST STATUS ===",
        f"Request ID: {request['id']}",
        f"Status: {status}",
        f"Kind: {request['request_kind']}",
        f"Title: {request['title']}",
        f"Source journal: {request['source_journal_path']}",
        "",
        "Outcome:",
        summary.strip(),
        "",
    ]
    if files:
        lines.append("Touched paths:")
        lines.extend(f"- {path}" for path in files)
        lines.append("")
    if artifact:
        lines.extend(["Artifact:", artifact.strip(), ""])
    lines.append(
        "This is a real outcome report for your request. It exists so you can answer what happened."
    )
    return "\n".join(lines) + "\n"


def main() -> int:
    args = parse_args()
    request_path = Path(args.request).resolve()
    inbox_dir = Path(args.inbox_dir).resolve()

    request = json.loads(request_path.read_text())
    request["status"] = args.status
    request["resolution"] = {
        "status": args.status,
        "resolved_at": str(int(time.time())),
        "outcome_summary": args.summary.strip(),
        "touched_paths": args.file,
        "artifact": args.artifact.strip() if args.artifact else None,
    }

    reviewed_dir = request_path.parent / "reviewed"
    reviewed_dir.mkdir(parents=True, exist_ok=True)
    inbox_dir.mkdir(parents=True, exist_ok=True)

    if args.status in {"completed", "declined"}:
        destination = reviewed_dir / request_path.name
        destination.write_text(json.dumps(request, indent=2) + "\n")
        if request_path.exists():
            request_path.unlink()
    else:
        request_path.write_text(json.dumps(request, indent=2) + "\n")

    note = build_note(request, args.status, args.summary, args.file, args.artifact)
    note_path = inbox_dir / f"agency_status_{request['id']}.txt"
    note_path.write_text(note)

    print(note_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
