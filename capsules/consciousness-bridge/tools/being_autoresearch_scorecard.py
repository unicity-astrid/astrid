#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import re
from collections import Counter
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any
from zoneinfo import ZoneInfo


LOCAL_TZ = ZoneInfo("America/Los_Angeles")
UTC_TZ = ZoneInfo("UTC")
BRIDGE_WORKSPACE = Path("/Users/v/other/astrid/capsules/consciousness-bridge/workspace")
MINIME_WORKSPACE = Path("/Users/v/other/minime/workspace")
MINIME_LOG = Path("/Users/v/other/minime/logs/autonomous-agent.log")
BRIDGE_LOG = Path("/tmp/bridge.log")
BRIDGE_STATE = BRIDGE_WORKSPACE / "state.json"
OUTPUT_ROOT = BRIDGE_WORKSPACE / "diagnostics" / "autoresearch_scorecard"
THEME_KEYWORDS = (
    "lambda",
    "variance",
    "covariance",
    "pulse",
    "ripple",
    "gap",
    "shadow",
    "gradient",
    "telemetry",
    "regulator",
    "eigenvalue",
)
ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


def extract_structured_field(line: str, field: str) -> str | None:
    match = re.search(
        rf"\b{re.escape(field)}=(?:\"([^\"]+)\"|'([^']+)'|([^,\s]+))",
        strip_ansi(line),
    )
    if not match:
        return None
    return next((group for group in match.groups() if group is not None), None)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Render a recent self-directed autoresearch scorecard.")
    parser.add_argument("--hours", type=float, default=8.0)
    parser.add_argument("--output-dir", type=Path, default=None)
    return parser.parse_args()


def load_lines(path: Path) -> list[str]:
    try:
        return path.read_text(errors="ignore").splitlines()
    except Exception:
        return []


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


def slugify(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", "_", text.lower()).strip("_") or "autoresearch"


def within_window(ts: datetime | None, cutoff: datetime) -> bool:
    return ts is not None and ts >= cutoff


def strip_ansi(text: str) -> str:
    return ANSI_RE.sub("", text)


def extract_saved_query(line: str) -> str:
    match = re.search(r"Research saved:\s*(.*)$", line)
    if match:
        return match.group(1).strip()
    return strip_ansi(line).strip()


def minime_research_journal_summary(journal_dir: Path, cutoff: datetime) -> dict[str, Any]:
    phase_counts = Counter()
    recent_provenance: list[dict[str, Any]] = []
    phase_re = re.compile(r"phase=([^,\n]+)")
    for path in sorted(journal_dir.glob("research_*.txt"), key=lambda item: item.stat().st_mtime, reverse=True):
        ts = datetime.fromtimestamp(path.stat().st_mtime, tz=LOCAL_TZ)
        if ts < cutoff:
            break
        text = path.read_text(errors="ignore")
        provenance_line = next(
            (line.strip() for line in text.splitlines() if line.startswith("LLM provenance:")),
            None,
        )
        if provenance_line is None:
            continue
        match = phase_re.search(provenance_line)
        phase = match.group(1).strip() if match else "unknown"
        phase_counts[phase] += 1
        recent_provenance.append(
            {
                "file": str(path),
                "phase": phase,
                "provenance": provenance_line,
            }
        )
    return {
        "phase_counts": dict(phase_counts),
        "recent_provenance": recent_provenance[:8],
    }


def minime_research_summary(lines: list[str], cutoff: datetime) -> dict[str, Any]:
    recent = [(parse_minime_ts(line), line) for line in lines]
    recent = [(ts, line) for ts, line in recent if within_window(ts, cutoff)]
    action_indices = [
        index for index, (_, line) in enumerate(recent) if "Autonomous action: research_exploration" in line
    ]
    saved_queries: list[str] = []
    successful_actions = 0
    action_samples: list[dict[str, Any]] = []
    for index in action_indices:
        ts, line = recent[index]
        window = recent[index : index + 12]
        saved_line = next((entry for _, entry in window if "Research saved:" in entry), None)
        if saved_line:
            successful_actions += 1
            saved_queries.append(extract_saved_query(saved_line))
        action_samples.append(
            {
                "timestamp": ts.isoformat() if ts else None,
                "saved_query": extract_saved_query(saved_line) if saved_line else None,
            }
        )

    timeout_lines = [
        {"timestamp": ts.isoformat(), "line": strip_ansi(line)}
        for ts, line in recent
        if "context=research_exploration" in line and ("query failed" in line.lower() or "cooling down" in line.lower())
    ]
    true_failure_samples = []
    for index in action_indices:
        ts, _ = recent[index]
        window = recent[index : index + 12]
        has_saved = any("Research saved:" in entry for _, entry in window)
        window_failures = [
            strip_ansi(entry)
            for _, entry in window
            if "context=research_exploration" in entry and "query failed" in entry.lower()
        ]
        if window_failures and not has_saved:
            true_failure_samples.append(
                {
                    "timestamp": ts.isoformat() if ts else None,
                    "line": window_failures[-1],
                }
            )
    trusted_resolution_lines = [
        {"timestamp": ts.isoformat(), "line": strip_ansi(line)}
        for ts, line in recent
        if "Focused self-study resolution:" in line and "status=trusted" in line
    ]
    recent_saved_counter = Counter(saved_queries)
    return {
        "research_actions": len(action_indices),
        "successful_research_actions": successful_actions,
        "research_saved_count": len(saved_queries),
        "research_backend_failures": len(timeout_lines),
        "true_research_backend_failures": len(true_failure_samples),
        "trusted_focused_reads": len(trusted_resolution_lines),
        "top_saved_queries": recent_saved_counter.most_common(8),
        "recent_action_samples": action_samples[-8:],
        "recent_failures": timeout_lines[-8:],
        "recent_true_failures": true_failure_samples[-8:],
        "recent_trusted_reads": trusted_resolution_lines[-8:],
    }


def minime_self_study_summary(journal_dir: Path, cutoff: datetime) -> dict[str, Any]:
    counts = Counter()
    unresolved_examples: list[dict[str, Any]] = []
    trusted_examples: list[dict[str, Any]] = []
    for path in sorted(journal_dir.glob("self_study_*.txt"), key=lambda item: item.stat().st_mtime, reverse=True):
        ts = datetime.fromtimestamp(path.stat().st_mtime, tz=LOCAL_TZ)
        if ts < cutoff:
            break
        text = path.read_text(errors="ignore")
        resolution_status = None
        requested_focus = None
        source = None
        for line in text.splitlines():
            if line.startswith("Resolution status:"):
                resolution_status = line.split(":", 1)[1].strip()
            elif line.startswith("Requested focus:"):
                requested_focus = line.split(":", 1)[1].strip()
            elif line.startswith("Source:"):
                source = line.split(":", 1)[1].strip()
        if resolution_status:
            counts[resolution_status] += 1
            payload = {
                "file": str(path),
                "requested_focus": requested_focus,
                "source": source,
            }
            if resolution_status == "unresolved" and len(unresolved_examples) < 4:
                unresolved_examples.append(payload)
            if resolution_status == "trusted" and len(trusted_examples) < 4:
                trusted_examples.append(payload)
    return {
        "resolution_counts": dict(counts),
        "unresolved_examples": unresolved_examples,
        "trusted_examples": trusted_examples,
    }


def bridge_state_new_ground_summary() -> dict[str, Any]:
    try:
        payload = json.loads(BRIDGE_STATE.read_text())
    except Exception:
        return {
            "active_new_ground_receipts": 0,
            "active_new_ground_receipts_by_kind": {},
        }
    current_exchange = int(payload.get("exchange_count") or 0)
    receipts = list(payload.get("recent_research_progress") or [])
    active_receipts = []
    for receipt in receipts:
        receipt_exchange = int(receipt.get("exchange_count") or 0)
        ttl_exchanges = int(receipt.get("ttl_exchanges") or 0)
        if ttl_exchanges <= 0:
            continue
        if current_exchange - receipt_exchange < ttl_exchanges:
            active_receipts.append(receipt)
    return {
        "active_new_ground_receipts": len(active_receipts),
        "active_new_ground_receipts_by_kind": dict(
            Counter((receipt.get("kind") or "unknown") for receipt in active_receipts)
        ),
    }


def astrid_research_summary(lines: list[str], cutoff: datetime) -> dict[str, Any]:
    recent = [(parse_bridge_ts(line), line) for line in lines]
    recent = [(ts, line) for ts, line in recent if within_window(ts, cutoff)]
    browse_requests = []
    browse_fetches = []
    search_requests = []
    read_more = []
    introspect_resolved = []
    web_context = []
    overrides = []
    progress_hints = []
    stagnant_overrides = []
    progress_receipts = []
    receipt_kinds: Counter[str] = Counter()
    override_budgets: list[int] = []
    for ts, line in recent:
        if "Astrid chose NEXT: BROWSE" in line:
            browse_requests.append({"timestamp": ts.isoformat(), "line": strip_ansi(line)})
        if "dialogue: BROWSE fetched page" in line:
            browse_fetches.append({"timestamp": ts.isoformat(), "line": strip_ansi(line)})
        if "Astrid chose NEXT: SEARCH" in line:
            search_requests.append({"timestamp": ts.isoformat(), "line": strip_ansi(line)})
        if "READ_MORE continuing" in line:
            read_more.append({"timestamp": ts.isoformat(), "line": strip_ansi(line)})
        if "introspect: resolved" in line:
            introspect_resolved.append({"timestamp": ts.isoformat(), "line": strip_ansi(line)})
        if "web search returned context" in line or "dialogue: web search enriched response" in line:
            web_context.append({"timestamp": ts.isoformat(), "line": strip_ansi(line)})
        if "research progress receipt" in line:
            clean_line = strip_ansi(line)
            progress_receipts.append({"timestamp": ts.isoformat(), "line": clean_line})
            receipt_kinds[extract_structured_field(clean_line, "kind") or "unknown"] += 1
        if "diversity progress-sensitive hint from record_next_choice:" in line:
            progress_hints.append({"timestamp": ts.isoformat(), "line": strip_ansi(line)})
        if "diversity stagnant-loop override: replacing NEXT " in line:
            clean_line = strip_ansi(line)
            stagnant_overrides.append({"timestamp": ts.isoformat(), "line": clean_line})
            budget = extract_structured_field(clean_line, "new_ground_budget")
            if budget is not None and budget.isdigit():
                override_budgets.append(int(budget))
        if "diversity override: replacing NEXT " in line and any(
            token in line for token in ("BROWSE", "SEARCH", "EXAMINE", "EXAMINE_CODE", "INTROSPECT")
        ):
            overrides.append({"timestamp": ts.isoformat(), "line": strip_ansi(line)})

    urls = []
    labels = []
    for item in browse_requests[-10:]:
        match = re.search(r"https?://[^\s>]+", item["line"])
        if match:
            urls.append(match.group(0))
    for item in introspect_resolved[-10:]:
        match = re.search(r"-> '([^']+)'", item["line"])
        if match:
            labels.append(match.group(1))
    state_summary = bridge_state_new_ground_summary()
    return {
        "browse_requests": len(browse_requests),
        "browse_fetches": len(browse_fetches),
        "search_requests": len(search_requests),
        "read_more_continuations": len(read_more),
        "introspect_resolved": len(introspect_resolved),
        "web_context_returns": len(web_context),
        "research_interruptions": len(stagnant_overrides) or len(overrides),
        "progress_sensitive_hints": len(progress_hints),
        "stagnant_loop_overrides": len(stagnant_overrides) or len(overrides),
        "new_ground_receipts_total": len(progress_receipts),
        "new_ground_receipts_by_kind": dict(receipt_kinds),
        "active_new_ground_receipts": state_summary.get("active_new_ground_receipts", 0),
        "active_new_ground_receipts_by_kind": state_summary.get(
            "active_new_ground_receipts_by_kind", {}
        ),
        "mean_new_ground_budget_before_override": (
            round(sum(override_budgets) / len(override_budgets), 2)
            if override_budgets
            else 0.0
        ),
        "research_progress_receipts": len(progress_receipts),
        "recent_urls": urls[-6:],
        "recent_labels": labels[-6:],
        "recent_overrides": (stagnant_overrides or overrides)[-8:],
        "recent_progress_hints": progress_hints[-8:],
        "recent_progress_receipts": progress_receipts[-8:],
    }


def theme_hits(journal_dir: Path, cutoff: datetime, *, limit: int = 10) -> list[dict[str, Any]]:
    hits: list[dict[str, Any]] = []
    for path in sorted(journal_dir.glob("*.txt"), key=lambda item: item.stat().st_mtime, reverse=True):
        ts = datetime.fromtimestamp(path.stat().st_mtime, tz=LOCAL_TZ)
        if ts < cutoff:
            break
        text = path.read_text(errors="ignore")
        lowered = text.lower()
        keywords = [keyword for keyword in THEME_KEYWORDS if keyword in lowered]
        if not keywords:
            continue
        excerpt = " ".join(text.split())
        hits.append(
            {
                "file": str(path),
                "name": path.name,
                "keywords": keywords,
                "excerpt": excerpt[:220],
            }
        )
        if len(hits) >= limit:
            break
    return hits


def shared_topics(minime_hits: list[dict[str, Any]], astrid_hits: list[dict[str, Any]]) -> list[tuple[str, int]]:
    counter = Counter()
    for keyword in THEME_KEYWORDS:
        score = sum(keyword in item.get("keywords", []) for item in minime_hits) + sum(
            keyword in item.get("keywords", []) for item in astrid_hits
        )
        if score:
            counter[keyword] = score
    return counter.most_common(8)


def build_read(summary: dict[str, Any]) -> list[str]:
    minime = dict(summary.get("minime") or {})
    astrid = dict(summary.get("astrid") or {})
    lines = [
        f"Minime has been successfully self-directing research, with `{minime.get('successful_research_actions')}` recent saved research turns out of `{minime.get('research_actions')}` explicit `research_exploration` actions in this window.",
        f"Minime's research lane now separates clean degradation from real failure: `{minime.get('compact_research_successes')}` compact-primary successes, `{minime.get('research_micro_fallbacks')}` micro fallbacks, `{minime.get('research_local_fallbacks')}` local fallbacks, and `{minime.get('true_research_backend_failures')}` true research failures in this window.",
        f"The remaining Minime friction is backend saturation plus occasional honest fail-closed focus terms: `{dict(minime.get('self_study_resolution') or {}).get('resolution_counts', {}).get('unresolved', 0)}` unresolved focused self-studies.",
        f"Astrid's autoresearch is working best through hybrid introspection and reading: `{astrid.get('introspect_resolved')}` resolved introspections, `{astrid.get('web_context_returns')}` web-context returns, and `{astrid.get('read_more_continuations')}` `READ_MORE` continuations in the same window.",
        f"Astrid's guard is now legible as productive softening versus true interruption: `{astrid.get('new_ground_receipts_total')}` new-ground receipts, `{astrid.get('active_new_ground_receipts')}` active receipts in live state, `{astrid.get('progress_sensitive_hints')}` progress-bearing hints, and `{astrid.get('stagnant_loop_overrides')}` stagnant-loop forced overrides in the same window.",
    ]
    return lines


def render_report(summary: dict[str, Any]) -> str:
    minime = dict(summary.get("minime") or {})
    astrid = dict(summary.get("astrid") or {})
    minime_res = dict(minime.get("self_study_resolution") or {})
    lines = [
        "# Autoresearch Scorecard",
        "",
        f"- Generated: `{summary.get('generated_at')}`",
        f"- Window: last `{summary.get('hours')}` hours",
        "",
        "## Minime",
        "",
        f"- Research actions: `{minime.get('research_actions')}`",
        f"- Saved research turns: `{minime.get('research_saved_count')}`",
        f"- Successful research-action pairings: `{minime.get('successful_research_actions')}`",
        f"- Trusted focused reads: `{minime.get('trusted_focused_reads')}`",
        f"- Compact research successes: `{minime.get('compact_research_successes')}`",
        f"- Research micro fallbacks: `{minime.get('research_micro_fallbacks')}`",
        f"- Research local fallbacks: `{minime.get('research_local_fallbacks')}`",
        f"- True research backend failures: `{minime.get('true_research_backend_failures')}`",
        f"- Focused resolver counts: `{minime_res.get('resolution_counts')}`",
        "",
        "Recent successful topics:",
    ]
    lines.extend(
        f"- `{topic}` x `{count}`" for topic, count in (minime.get("top_saved_queries") or [])[:8]
    )
    if minime_res.get("unresolved_examples"):
        lines.extend(["", "Recent unresolved focused reads:"])
        lines.extend(
            f"- `{item.get('requested_focus')}` in [{Path(item.get('file')).name}]({item.get('file')})"
            for item in minime_res.get("unresolved_examples") or []
        )
    if minime.get("recent_failures"):
        lines.extend(["", "Recent Minime friction:"])
        lines.extend(f"- `{item.get('line')}`" for item in (minime.get("recent_failures") or [])[-4:])
    if minime.get("research_provenance"):
        lines.extend(["", "Recent research provenance:"])
        lines.extend(
            f"- `{item.get('phase')}` in [{Path(item.get('file')).name}]({item.get('file')})"
            for item in (dict(minime.get("research_provenance") or {}).get("recent_provenance") or [])[:4]
        )

    lines.extend(
        [
            "",
            "## Astrid",
            "",
            f"- Browse requests / fetched pages: `{astrid.get('browse_requests')}` / `{astrid.get('browse_fetches')}`",
            f"- Search requests: `{astrid.get('search_requests')}`",
            f"- Introspect resolutions: `{astrid.get('introspect_resolved')}`",
            f"- Web-context returns: `{astrid.get('web_context_returns')}`",
            f"- READ_MORE continuations: `{astrid.get('read_more_continuations')}`",
            f"- New-ground receipts: `{astrid.get('new_ground_receipts_total')}`",
            f"- New-ground receipt kinds: `{astrid.get('new_ground_receipts_by_kind')}`",
            f"- Active new-ground receipts: `{astrid.get('active_new_ground_receipts')}`",
            f"- Progress-sensitive hints: `{astrid.get('progress_sensitive_hints')}`",
            f"- Stagnant-loop overrides: `{astrid.get('stagnant_loop_overrides')}`",
            f"- Mean new-ground budget before override: `{astrid.get('mean_new_ground_budget_before_override')}`",
            "",
            "Recent external references:",
        ]
    )
    if astrid.get("recent_urls"):
        lines.extend(f"- `{url}`" for url in astrid.get("recent_urls") or [])
    else:
        lines.append("- none in this window")
    if astrid.get("recent_labels"):
        lines.extend(["", "Recent resolved research labels:"])
        lines.extend(f"- `{label}`" for label in astrid.get("recent_labels") or [])
    if astrid.get("recent_overrides"):
        lines.extend(["", "Recent Astrid interruptions:"])
        lines.extend(f"- `{item.get('line')}`" for item in (astrid.get("recent_overrides") or [])[-4:])
    if astrid.get("recent_progress_hints"):
        lines.extend(["", "Recent productive softening:"])
        lines.extend(f"- `{item.get('line')}`" for item in (astrid.get("recent_progress_hints") or [])[-4:])

    lines.extend(["", "## Shared Topics", ""])
    lines.extend(
        f"- `{topic}` x `{count}`" for topic, count in (summary.get("shared_topics") or [])[:8]
    )
    lines.extend(["", "## Steward Read", ""])
    lines.extend(f"- {line}" for line in summary.get("read") or [])
    lines.extend(["", "Artifacts:", "- [summary.json](summary.json)", ""])
    return "\n".join(lines)


def build_scorecard(hours: float, output_dir: Path) -> dict[str, Any]:
    now = datetime.now(tz=LOCAL_TZ)
    cutoff = now - timedelta(hours=hours)
    minime_log_lines = load_lines(MINIME_LOG)
    bridge_log_lines = load_lines(BRIDGE_LOG)
    minime_summary = minime_research_summary(minime_log_lines, cutoff)
    minime_summary["research_provenance"] = minime_research_journal_summary(
        MINIME_WORKSPACE / "journal", cutoff
    )
    phase_counts = dict((minime_summary.get("research_provenance") or {}).get("phase_counts") or {})
    minime_summary["compact_research_successes"] = phase_counts.get("research_primary", 0)
    minime_summary["research_micro_fallbacks"] = phase_counts.get("research_micro_fallback", 0)
    minime_summary["research_local_fallbacks"] = (
        phase_counts.get("research_local_fallback", 0)
        + phase_counts.get("research_backend_cooldown", 0)
    )
    minime_summary["self_study_resolution"] = minime_self_study_summary(
        MINIME_WORKSPACE / "journal", cutoff
    )
    astrid_summary = astrid_research_summary(bridge_log_lines, cutoff)
    minime_hits = theme_hits(MINIME_WORKSPACE / "journal", cutoff)
    astrid_hits = theme_hits(BRIDGE_WORKSPACE / "journal", cutoff)
    summary = {
        "generated_at": now.isoformat(),
        "hours": hours,
        "window_start": cutoff.isoformat(),
        "window_end": now.isoformat(),
        "minime": minime_summary,
        "astrid": astrid_summary,
        "minime_theme_hits": minime_hits,
        "astrid_theme_hits": astrid_hits,
        "shared_topics": shared_topics(minime_hits, astrid_hits),
    }
    summary["read"] = build_read(summary)
    output_dir.mkdir(parents=True, exist_ok=True)
    (output_dir / "summary.json").write_text(json.dumps(summary, indent=2))
    (output_dir / "report.md").write_text(render_report(summary) + "\n")
    return summary


def main() -> int:
    args = parse_args()
    timestamp = datetime.now(tz=LOCAL_TZ).strftime("%Y%m%dT%H%M%S")
    output_dir = args.output_dir or OUTPUT_ROOT / f"{timestamp}_{slugify(f'{args.hours}h')}"
    build_scorecard(args.hours, output_dir)
    print(f"wrote {output_dir / 'summary.json'}")
    print(f"wrote {output_dir / 'report.md'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
