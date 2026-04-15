#!/usr/bin/env python3

from __future__ import annotations

import argparse
import subprocess
import time
from datetime import datetime
from pathlib import Path
from zoneinfo import ZoneInfo


LOCAL_TZ = ZoneInfo("America/Los_Angeles")
TOOL = Path("/Users/v/other/astrid/capsules/consciousness-bridge/tools/being_action_canonicalization_scorecard.py")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the action canonicalization scorecard at a later local time.")
    parser.add_argument("--run-at", required=True, help="ISO8601 local/offset timestamp for the later run.")
    parser.add_argument("--since", required=True, help="ISO8601 local/offset timestamp for the scan window start.")
    parser.add_argument("--output-dir", required=True)
    parser.add_argument("--topic", default="later_post_deploy")
    parser.add_argument("--log-path", default="/tmp/action_canonicalization_scorecard_evening_runner.log")
    return parser.parse_args()


def parse_localish(raw: str) -> datetime:
    parsed = datetime.fromisoformat(raw)
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=LOCAL_TZ)
    return parsed.astimezone(LOCAL_TZ)


def main() -> None:
    args = parse_args()
    now = datetime.now(LOCAL_TZ)
    target = parse_localish(args.run_at)
    sleep_seconds = max(0.0, (target - now).total_seconds())
    log_path = Path(args.log_path)
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("a", encoding="utf-8") as fh:
        fh.write(
            f"started_at={now.isoformat()} target={target.isoformat()} sleep_seconds={sleep_seconds:.0f}\n"
        )
    time.sleep(sleep_seconds)
    cmd = [
        "python3",
        str(TOOL),
        "--since",
        args.since,
        "--topic",
        args.topic,
        "--output-dir",
        args.output_dir,
    ]
    completed = subprocess.run(cmd, check=False, capture_output=True, text=True)
    with log_path.open("a", encoding="utf-8") as fh:
        fh.write(f"completed_at={datetime.now(LOCAL_TZ).isoformat()} rc={completed.returncode}\n")
        if completed.stdout:
            fh.write(f"stdout={completed.stdout.strip()}\n")
        if completed.stderr:
            fh.write(f"stderr={completed.stderr.strip()}\n")


if __name__ == "__main__":
    main()
