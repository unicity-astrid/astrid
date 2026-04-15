#!/usr/bin/env python3

from __future__ import annotations

import argparse
import csv
import json
from collections import Counter
from datetime import datetime
from pathlib import Path
from typing import Any


BRIDGE_ROOT = Path("/Users/v/other/astrid/capsules/consciousness-bridge")
BRIDGE_WORKSPACE = BRIDGE_ROOT / "workspace"
DEFAULT_RUNS_ROOT = BRIDGE_WORKSPACE / "diagnostics" / "being_phase_transition_edges"
DEFAULT_OUTPUT_DIR = DEFAULT_RUNS_ROOT / "archive"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build an archive index for being phase-transition edge bundles."
    )
    parser.add_argument(
        "--runs-root",
        type=Path,
        default=DEFAULT_RUNS_ROOT,
        help="Directory containing transition-edge watcher bundles.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=DEFAULT_OUTPUT_DIR,
        help="Archive output directory.",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=0,
        help="Optional limit on most-recent bundles to include.",
    )
    return parser.parse_args()


def ensure_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)


def load_json(path: Path) -> dict[str, Any]:
    try:
        return json.loads(path.read_text())
    except Exception:
        return {}


def safe_float(value: Any, default: float = 0.0) -> float:
    try:
        return float(value)
    except (TypeError, ValueError):
        return default


def safe_relpath(target: Path, start: Path) -> str:
    try:
        return target.relative_to(start).as_posix()
    except ValueError:
        return target.resolve().as_posix()


def discover_bundles(runs_root: Path, limit: int, output_dir: Path) -> list[Path]:
    if not runs_root.exists():
        return []
    bundles = [
        path
        for path in runs_root.iterdir()
        if path.is_dir()
        and path != output_dir
        and path.name != output_dir.name
        and (path / "summary.json").exists()
    ]
    bundles.sort(key=lambda path: path.name)
    if limit > 0:
        bundles = bundles[-limit:]
    return bundles


def derive_tags(summary: dict[str, Any]) -> list[str]:
    tags: list[str] = []
    event = dict(summary.get("event") or {})
    payload = dict(event.get("event_payload") or {})
    current = dict(event.get("current") or {})
    before_minime = dict(summary.get("before_minime") or {})
    lag = dict(summary.get("cross_being_lag") or {})
    journals = dict(summary.get("journals") or {})
    delayed = dict(summary.get("delayed") or {})
    immediate = dict(summary.get("immediate") or {})
    context_overlay = dict(summary.get("astrid_context_overlay") or {})
    fill_band = payload.get("fill_band") or current.get("fill_band")
    edge_kind = str(event.get("kind") or "")
    delayed_fill = safe_float(delayed.get("delta_fill_pct"))
    immediate_fill = safe_float(immediate.get("delta_fill_pct"))

    if fill_band == "under":
        tags.append("underfill")
    elif fill_band == "over":
        tags.append("overfill")
    else:
        tags.append("near_target")

    if edge_kind:
        tags.append(edge_kind)
    if edge_kind == "phase_transition" and payload.get("phase_to") == "plateau":
        tags.append("settling_edge")
    if fill_band == "under" and delayed_fill > 4.0:
        tags.append("underfill_recovery")
    if fill_band == "over" and delayed_fill < -4.0:
        tags.append("pressure_release")
    if immediate_fill > 0.0 and delayed_fill > immediate_fill + 4.0:
        tags.append("continued_recovery")
    if str(lag.get("classification") or "") == "delayed_astrid_response":
        tags.append("delayed_astrid_follow_on")
    if str(lag.get("classification") or "") == "minimal_follow_on":
        tags.append("one_sided_transition")
    if any(path.startswith("astrid/") for path in journals.get("new_after_delayed") or []):
        tags.append("delayed_astrid_journal")
    if any(path.startswith("astrid/") for path in journals.get("new_after_immediate") or []):
        tags.append("immediate_astrid_journal")
    if any("perturb_" in path for path in journals.get("before") or []):
        tags.append("post_perturb_context")
    if any("self_study_" in path for path in journals.get("before") or []):
        tags.append("self_study_context")
    if current.get("both_backends_hot") or edge_kind == "backend_hot":
        tags.append("backend_pressure")
    before_quadrant = str(before_minime.get("dominant_quadrant") or "")
    delayed_quadrant = str(delayed.get("after_dominant_quadrant") or "")
    if before_quadrant and delayed_quadrant and before_quadrant != delayed_quadrant:
        tags.append("quadrant_flip")
    if safe_float(delayed.get("after_mean_radius")) <= safe_float(before_minime.get("mean_radius")) - 0.10:
        if safe_float(delayed.get("after_latent_span")) < safe_float(before_minime.get("latent_span")) * 0.85:
            tags.append("tightening_arc")
        if safe_float(delayed.get("internal_process_x_delta")) >= 0.15:
            tags.append("reopening_arc")
    if safe_float(delayed.get("latent_centroid_shift")) > 0.35:
        tags.append("basin_shift")
    covariance_mode = str(delayed.get("after_covariance_mode") or immediate.get("after_covariance_mode") or "")
    if covariance_mode == "reinforced":
        tags.append("covariance_reinforcement")
    elif covariance_mode == "relaxed":
        tags.append("covariance_relaxation")
    floor_share = safe_float(
        delayed.get("after_floor_support_share") or immediate.get("after_floor_support_share")
    )
    if floor_share >= 0.25 and delayed_fill > 0.0:
        tags.append("floor_supported_recovery")
    perturb_effect = str(
        delayed.get("after_perturb_effect") or immediate.get("after_perturb_effect") or ""
    )
    perturb_response = str(
        delayed.get("after_perturb_response_balance")
        or immediate.get("after_perturb_response_balance")
        or ""
    )
    if perturb_effect in {"opened", "softened_only"}:
        tags.append("perturb_visible_shift")
    if perturb_effect == "softened_only" or perturb_response == "recovery_without_widening":
        tags.append("softening_without_opening")
    if context_overlay.get("shared_covariance_theme"):
        tags.append("shared_covariance_theme")
    return sorted(set(tags))


def derive_row(bundle_dir: Path, output_dir: Path) -> dict[str, Any]:
    summary = load_json(bundle_dir / "summary.json")
    event = dict(summary.get("event") or {})
    payload = dict(event.get("event_payload") or {})
    lag = dict(summary.get("cross_being_lag") or {})
    immediate = dict(summary.get("immediate") or {})
    delayed = dict(summary.get("delayed") or {})
    journals = dict(summary.get("journals") or {})
    row = {
        "bundle": bundle_dir.name,
        "generated_at": summary.get("generated_at") or bundle_dir.name,
        "label": summary.get("label") or bundle_dir.name,
        "edge_kind": event.get("kind"),
        "trigger_mode": event.get("trigger_mode"),
        "confidence": event.get("confidence"),
        "phase_from": payload.get("phase_from"),
        "phase_to": payload.get("phase_to"),
        "fill_band": payload.get("fill_band") or dict(event.get("current") or {}).get("fill_band"),
        "before_fill_pct": safe_float(summary.get("before_fill_pct")),
        "before_quadrant": dict(summary.get("before_minime") or {}).get("dominant_quadrant"),
        "immediate_fill_delta": safe_float(immediate.get("delta_fill_pct")),
        "delayed_fill_delta": safe_float(delayed.get("delta_fill_pct")),
        "delayed_quadrant": delayed.get("after_dominant_quadrant"),
        "delayed_mean_radius": safe_float(delayed.get("after_mean_radius")),
        "delayed_latent_span": safe_float(delayed.get("after_latent_span")),
        "latent_centroid_shift": safe_float(delayed.get("latent_centroid_shift")),
        "lag_class": lag.get("classification"),
        "lag_score": safe_float(lag.get("lag_score")),
        "astrid_delayed_share": safe_float(lag.get("astrid_delayed_share")),
        "immediate_coupling_ratio": safe_float(lag.get("immediate_coupling_ratio")),
        "delayed_coupling_ratio": safe_float(lag.get("delayed_coupling_ratio")),
        "new_immediate_files": len(journals.get("new_after_immediate") or []),
        "new_delayed_files": len(journals.get("new_after_delayed") or []),
        "report": safe_relpath(bundle_dir / "report.md", output_dir),
        "summary": safe_relpath(bundle_dir / "summary.json", output_dir),
        "unique_signals": list(summary.get("unique_signals") or []),
    }
    row["tags"] = derive_tags(summary)
    return row


def top_rows(rows: list[dict[str, Any]], *, key: str, reverse: bool = True, limit: int = 5) -> list[dict[str, Any]]:
    return sorted(rows, key=lambda row: safe_float(row.get(key)), reverse=reverse)[:limit]


def write_csv(rows: list[dict[str, Any]], path: Path) -> None:
    fieldnames = [
        "bundle",
        "generated_at",
        "label",
        "edge_kind",
        "trigger_mode",
        "confidence",
        "phase_from",
        "phase_to",
        "fill_band",
        "before_fill_pct",
        "before_quadrant",
        "immediate_fill_delta",
        "delayed_fill_delta",
        "delayed_quadrant",
        "delayed_mean_radius",
        "delayed_latent_span",
        "latent_centroid_shift",
        "lag_class",
        "lag_score",
        "astrid_delayed_share",
        "immediate_coupling_ratio",
        "delayed_coupling_ratio",
        "new_immediate_files",
        "new_delayed_files",
        "tags",
        "report",
        "summary",
    ]
    with path.open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fieldnames)
        writer.writeheader()
        for row in rows:
            data = dict(row)
            data["tags"] = ",".join(row.get("tags") or [])
            data.pop("unique_signals", None)
            writer.writerow(data)


def write_report(output_dir: Path, summary: dict[str, Any], rows: list[dict[str, Any]]) -> None:
    report = [
        "# Transition Edge Archive",
        "",
        f"Generated: `{summary['generated_at']}`",
        f"Bundles indexed: `{summary['bundle_count']}`",
        "",
        "## Lag Classes",
        "",
    ]
    for key, count in summary.get("lag_class_counts", {}).items():
        report.append(f"- `{key}`: `{count}`")
    report.extend(["", "## Context Tags", ""])
    for key, count in summary.get("tag_counts", {}).items():
        report.append(f"- `{key}`: `{count}`")
    report.extend(["", "## Most Delayed Astrid Follow-On", ""])
    for row in summary.get("top_delayed_astrid", []):
        report.append(
            f"- `{row['bundle']}` lag `{safe_float(row['lag_score']):+.3f}` delayed share `{safe_float(row['astrid_delayed_share']):.3f}` [{row['edge_kind']}]({row['report']})"
        )
    report.extend(["", "## Strongest Immediate Astrid Responses", ""])
    for row in summary.get("top_immediate_astrid", []):
        report.append(
            f"- `{row['bundle']}` lag `{safe_float(row['lag_score']):+.3f}` delayed share `{safe_float(row['astrid_delayed_share']):.3f}` [{row['edge_kind']}]({row['report']})"
        )
    report.extend(["", "## Recovery-Favored Edges", ""])
    for row in summary.get("top_recovery_edges", []):
        report.append(
            f"- `{row['bundle']}` delayed fill `{safe_float(row['delayed_fill_delta']):+.2f}` tags `{', '.join(row.get('tags') or [])}` [{row['edge_kind']}]({row['report']})"
        )
    report.extend(["", "## Recent Bundles", ""])
    for row in rows[-8:]:
        report.append(
            f"- `{row['bundle']}` `{row['edge_kind']}` lag `{safe_float(row['lag_score']):+.3f}` tags `{', '.join(row.get('tags') or [])}`"
        )
    report.extend(
        [
            "",
            "## Artifacts",
            "",
            "- [summary.json](summary.json)",
            "- [transitions.csv](transitions.csv)",
        ]
    )
    (output_dir / "report.md").write_text("\n".join(report) + "\n")


def main() -> int:
    args = parse_args()
    ensure_dir(args.output_dir)
    bundles = discover_bundles(args.runs_root, args.limit, args.output_dir)
    rows = [derive_row(bundle, args.output_dir) for bundle in bundles]
    rows.sort(key=lambda row: str(row.get("generated_at")))
    lag_counts = Counter(str(row.get("lag_class") or "unknown") for row in rows)
    tag_counts = Counter(tag for row in rows for tag in row.get("tags") or [])
    summary = {
        "generated_at": datetime.now().isoformat(),
        "bundle_count": len(rows),
        "runs_root": args.runs_root.resolve().as_posix(),
        "lag_class_counts": dict(sorted(lag_counts.items())),
        "tag_counts": dict(sorted(tag_counts.items())),
        "top_delayed_astrid": top_rows(rows, key="lag_score", reverse=True),
        "top_immediate_astrid": top_rows(rows, key="lag_score", reverse=False),
        "top_recovery_edges": top_rows(rows, key="delayed_fill_delta", reverse=True),
    }
    (args.output_dir / "summary.json").write_text(json.dumps(summary, indent=2))
    write_csv(rows, args.output_dir / "transitions.csv")
    write_report(args.output_dir, summary, rows)
    print(f"wrote {args.output_dir / 'summary.json'}")
    print(f"wrote {args.output_dir / 'transitions.csv'}")
    print(f"wrote {args.output_dir / 'report.md'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
