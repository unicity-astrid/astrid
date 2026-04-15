#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import re
from collections import Counter
from dataclasses import dataclass
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any
from zoneinfo import ZoneInfo


LOCAL_TZ = ZoneInfo("America/Los_Angeles")
UTC_TZ = ZoneInfo("UTC")
BRIDGE_WORKSPACE = Path("/Users/v/other/astrid/capsules/consciousness-bridge/workspace")
MINIME_LOG = Path("/Users/v/other/minime/logs/autonomous-agent.log")
BRIDGE_LOG = Path("/tmp/bridge.log")
OUTPUT_ROOT = BRIDGE_WORKSPACE / "diagnostics" / "action_canonicalization_scorecard"
ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


@dataclass(frozen=True)
class FormRule:
    system: str
    form: str
    target: re.Pattern[str]
    unknown: re.Pattern[str] | None
    rescue: tuple[re.Pattern[str], ...]
    guarded: tuple[re.Pattern[str], ...] = ()


MINIME_RULES = (
    FormRule(
        system="minime",
        form="deep_read",
        target=re.compile(r"Being chose NEXT: DEEP_READ\b", re.IGNORECASE),
        unknown=re.compile(r"Unknown NEXT: 'DEEP_READ\b", re.IGNORECASE),
        rescue=(
            re.compile(r"Honoring being's NEXT: .*→ read_more", re.IGNORECASE),
            re.compile(r"Honoring being's NEXT: READ_MORE .*→ browse_url", re.IGNORECASE),
        ),
    ),
    FormRule(
        system="minime",
        form="explain",
        target=re.compile(r"Being chose NEXT: EXPLAIN\b", re.IGNORECASE),
        unknown=re.compile(r"Unknown NEXT: 'EXPLAIN\b", re.IGNORECASE),
        rescue=(
            re.compile(r"Honoring being's NEXT: SELF_STUDY '.*' → self_study", re.IGNORECASE),
        ),
    ),
    FormRule(
        system="minime",
        form="code_start",
        target=re.compile(r"Being chose NEXT: CODE_START\b", re.IGNORECASE),
        unknown=re.compile(r"Unknown NEXT: 'CODE_START\b", re.IGNORECASE),
        rescue=(
            re.compile(r"Honoring being's NEXT: EXAMINE_CODE '.*' → self_study", re.IGNORECASE),
        ),
    ),
)

ASTRID_RULES = (
    FormRule(
        system="astrid",
        form="bracketed_experiment_run",
        target=re.compile(r"Astrid chose NEXT: \[EXPERIMENT_RUN\b", re.IGNORECASE),
        unknown=re.compile(r"Astrid chose unknown NEXT: '\[EXPERIMENT_RUN\b", re.IGNORECASE),
        rescue=(re.compile(r"\bEXPERIMENT_RUN:\s+", re.IGNORECASE),),
        guarded=(re.compile(r"diversity .*NEXT .*EXPERIMENT_RUN", re.IGNORECASE),),
    ),
    FormRule(
        system="astrid",
        form="experiment_run_markers",
        target=re.compile(
            r"Astrid chose NEXT: EXPERIMENT_RUN\s+(?:-ws|workspace_name:|workspace=|workspace:|ws=|ws:)",
            re.IGNORECASE,
        ),
        unknown=re.compile(
            r"Astrid chose unknown NEXT: 'EXPERIMENT_RUN\s+(?:-ws|workspace_name:|workspace=|workspace:|ws=|ws:)",
            re.IGNORECASE,
        ),
        rescue=(re.compile(r"\bEXPERIMENT_RUN:\s+", re.IGNORECASE),),
        guarded=(re.compile(r"diversity .*NEXT .*EXPERIMENT_RUN", re.IGNORECASE),),
    ),
    FormRule(
        system="astrid",
        form="gesture_alias",
        target=re.compile(r"Astrid chose NEXT: GESTURE_[A-Z_]+\b"),
        unknown=re.compile(r"Astrid chose unknown NEXT: 'GESTURE_[A-Z_]+\b"),
        rescue=(re.compile(r"Astrid sent spectral gesture:", re.IGNORECASE),),
        guarded=(re.compile(r"diversity .*NEXT .*GESTURE", re.IGNORECASE),),
    ),
    FormRule(
        system="astrid",
        form="gesture_call_wrapper",
        target=re.compile(r"Astrid chose NEXT: GESTURE\(", re.IGNORECASE),
        unknown=re.compile(r"Astrid chose unknown NEXT: 'GESTURE\(", re.IGNORECASE),
        rescue=(re.compile(r"Astrid sent spectral gesture:", re.IGNORECASE),),
        guarded=(re.compile(r"diversity .*NEXT .*GESTURE", re.IGNORECASE),),
    ),
    FormRule(
        system="astrid",
        form="code_wrapper",
        target=re.compile(r"Astrid chose NEXT: EXAMINE_CODE .*(?: - | — )", re.IGNORECASE),
        unknown=re.compile(r"Astrid chose unknown NEXT: 'EXAMINE_CODE .*(?: - | — )", re.IGNORECASE),
        rescue=(re.compile(r"Astrid chose EXAMINE_CODE:", re.IGNORECASE),),
        guarded=(re.compile(r"diversity .*NEXT EXAMINE_CODE", re.IGNORECASE),),
    ),
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Scan post-deploy logs for rescued NEXT-action forms and classify whether they reappeared cleanly."
    )
    parser.add_argument("--since", type=str, default=None, help="ISO8601 local or offset timestamp.")
    parser.add_argument("--hours", type=float, default=None, help="Alternative lookback window if --since is omitted.")
    parser.add_argument("--output-dir", type=Path, default=None)
    parser.add_argument("--topic", type=str, default="post_deploy")
    return parser.parse_args()


def strip_ansi(text: str) -> str:
    return ANSI_RE.sub("", text)


def slugify(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", "_", text.lower()).strip("_") or "scorecard"


def parse_since(raw: str | None, hours: float | None) -> datetime:
    if raw:
        parsed = datetime.fromisoformat(raw)
        if parsed.tzinfo is None:
            return parsed.replace(tzinfo=LOCAL_TZ)
        return parsed.astimezone(LOCAL_TZ)
    lookback_hours = 8.0 if hours is None else max(hours, 0.0)
    return datetime.now(tz=LOCAL_TZ) - timedelta(hours=lookback_hours)


def parse_minime_ts(line: str) -> datetime | None:
    match = re.match(r"^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})", line)
    if not match:
        return None
    try:
        return datetime.strptime(match.group(1), "%Y-%m-%d %H:%M:%S").replace(tzinfo=LOCAL_TZ)
    except ValueError:
        return None


def parse_bridge_ts(line: str) -> datetime | None:
    match = re.match(r"^\x1b\[2m(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?)Z", line)
    if not match:
        return None
    try:
        return datetime.fromisoformat(match.group(1)).replace(tzinfo=UTC_TZ).astimezone(LOCAL_TZ)
    except ValueError:
        return None


def load_recent_entries(path: Path, parser, cutoff: datetime) -> list[tuple[datetime, str]]:
    entries: list[tuple[datetime, str]] = []
    try:
        lines = path.read_text(errors="ignore").splitlines()
    except Exception:
        return entries
    for line in lines:
        ts = parser(line)
        if ts is None or ts < cutoff:
            continue
        entries.append((ts, strip_ansi(line)))
    return entries


def classify_events(entries: list[tuple[datetime, str]], rules: tuple[FormRule, ...]) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    for index, (ts, line) in enumerate(entries):
        for rule in rules:
            if not rule.target.search(line):
                continue
            status = "seen_only"
            evidence = None
            for _, follow_line in entries[index + 1 : index + 11]:
                if rule.unknown and rule.unknown.search(follow_line):
                    status = "unknown"
                    evidence = follow_line
                    break
                if any(pattern.search(follow_line) for pattern in rule.rescue):
                    status = "rescued"
                    evidence = follow_line
                    break
                if any(pattern.search(follow_line) for pattern in rule.guarded):
                    status = "guarded"
                    evidence = follow_line
                    break
            events.append(
                {
                    "system": rule.system,
                    "form": rule.form,
                    "timestamp": ts.isoformat(),
                    "line": line,
                    "status": status,
                    "evidence": evidence,
                }
            )
            break
    return events


def summarize_system(events: list[dict[str, Any]], forms: tuple[FormRule, ...]) -> dict[str, Any]:
    form_counts: dict[str, dict[str, int]] = {}
    status_counts = Counter()
    for rule in forms:
        relevant = [event for event in events if event["form"] == rule.form]
        counts = Counter(event["status"] for event in relevant)
        status_counts.update(counts)
        form_counts[rule.form] = {
            "appeared": len(relevant),
            "rescued": counts.get("rescued", 0),
            "guarded": counts.get("guarded", 0),
            "unknown": counts.get("unknown", 0),
            "seen_only": counts.get("seen_only", 0),
        }
    return {
        "forms": form_counts,
        "status_counts": dict(status_counts),
        "recent_events": events[-10:],
    }


def steward_read(minime: dict[str, Any], astrid: dict[str, Any]) -> list[str]:
    lines: list[str] = []

    minime_rescued = sum(item["rescued"] for item in minime["forms"].values())
    minime_unknown = sum(item["unknown"] for item in minime["forms"].values())
    minime_total = sum(item["appeared"] for item in minime["forms"].values())
    if minime_total == 0:
        lines.append("Minime did not organically reuse the rescued DEEP_READ / EXPLAIN / CODE_START forms in this window.")
    elif minime_unknown == 0:
        lines.append(
            f"Minime reused {minime_total} targeted rescued-form action(s) with {minime_rescued} clean routed outcome(s) and no post-deploy unknowns."
        )
    else:
        lines.append(
            f"Minime reused {minime_total} targeted rescued-form action(s), but {minime_unknown} still fell through as unknown and needs another pass."
        )

    astrid_rescued = sum(item["rescued"] for item in astrid["forms"].values())
    astrid_guarded = sum(item["guarded"] for item in astrid["forms"].values())
    astrid_unknown = sum(item["unknown"] for item in astrid["forms"].values())
    astrid_total = sum(item["appeared"] for item in astrid["forms"].values())
    if astrid_total == 0:
        lines.append("Astrid did not organically reuse the rescued bracketed experiment / gesture-wrapper forms in this window.")
    elif astrid_unknown == 0:
        lines.append(
            f"Astrid reused {astrid_total} targeted rescued-form action(s) with {astrid_rescued} explicit routed execution(s) and {astrid_guarded} guard-mediated continuation(s), with no new unknowns."
        )
    else:
        lines.append(
            f"Astrid reused {astrid_total} targeted rescued-form action(s), but {astrid_unknown} still surfaced as unknown and should be re-checked."
        )

    if minime_unknown == 0 and astrid_unknown == 0:
        lines.append("The post-deploy action-language floor looks healthier: the rescued forms are either absent or staying inside the wired ecology instead of leaking into unknown-action logs.")
    return lines


def render_report(
    output_dir: Path,
    cutoff: datetime,
    minime_summary: dict[str, Any],
    astrid_summary: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    lines = [
        "# Action Canonicalization Scorecard",
        "",
        f"- Scan start: `{cutoff.isoformat()}`",
        f"- Generated: `{datetime.now(tz=LOCAL_TZ).isoformat()}`",
        "",
        "## Steward Read",
        "",
    ]
    for line in summary["steward_read"]:
        lines.append(f"- {line}")

    lines.extend(["", "## Minime", ""])
    for form, counts in minime_summary["forms"].items():
        lines.append(
            f"- `{form}`: appeared={counts['appeared']}, rescued={counts['rescued']}, guarded={counts['guarded']}, unknown={counts['unknown']}, seen_only={counts['seen_only']}"
        )
    if minime_summary["recent_events"]:
        lines.extend(["", "Recent Minime events:"])
        for event in minime_summary["recent_events"]:
            lines.append(f"- `{event['timestamp']}` `{event['form']}` `{event['status']}`")
            lines.append(f"  Source: {event['line']}")
            if event["evidence"]:
                lines.append(f"  Evidence: {event['evidence']}")

    lines.extend(["", "## Astrid", ""])
    for form, counts in astrid_summary["forms"].items():
        lines.append(
            f"- `{form}`: appeared={counts['appeared']}, rescued={counts['rescued']}, guarded={counts['guarded']}, unknown={counts['unknown']}, seen_only={counts['seen_only']}"
        )
    if astrid_summary["recent_events"]:
        lines.extend(["", "Recent Astrid events:"])
        for event in astrid_summary["recent_events"]:
            lines.append(f"- `{event['timestamp']}` `{event['form']}` `{event['status']}`")
            lines.append(f"  Source: {event['line']}")
            if event["evidence"]:
                lines.append(f"  Evidence: {event['evidence']}")

    (output_dir / "report.md").write_text("\n".join(lines).strip() + "\n")


def main() -> None:
    args = parse_args()
    cutoff = parse_since(args.since, args.hours)
    output_dir = args.output_dir
    if output_dir is None:
        stamp = datetime.now(tz=LOCAL_TZ).strftime("%Y%m%dT%H%M%S")
        output_dir = OUTPUT_ROOT / f"{stamp}_{slugify(args.topic)}"
    output_dir.mkdir(parents=True, exist_ok=True)

    minime_entries = load_recent_entries(MINIME_LOG, parse_minime_ts, cutoff)
    bridge_entries = load_recent_entries(BRIDGE_LOG, parse_bridge_ts, cutoff)
    minime_events = classify_events(minime_entries, MINIME_RULES)
    astrid_events = classify_events(bridge_entries, ASTRID_RULES)
    minime_summary = summarize_system(minime_events, MINIME_RULES)
    astrid_summary = summarize_system(astrid_events, ASTRID_RULES)
    summary = {
        "scan_start": cutoff.isoformat(),
        "generated_at": datetime.now(tz=LOCAL_TZ).isoformat(),
        "topic": args.topic,
        "minime": minime_summary,
        "astrid": astrid_summary,
        "steward_read": steward_read(minime_summary, astrid_summary),
    }

    (output_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    render_report(output_dir, cutoff, minime_summary, astrid_summary, summary)
    print(output_dir)


if __name__ == "__main__":
    main()
