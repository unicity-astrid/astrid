#!/usr/bin/env python3

from __future__ import annotations

import argparse
import csv
import json
from datetime import datetime
from pathlib import Path
from typing import Any


BRIDGE_ROOT = Path("/Users/v/other/astrid/capsules/consciousness-bridge")
BRIDGE_WORKSPACE = BRIDGE_ROOT / "workspace"
BRIDGE_DIAGNOSTICS = BRIDGE_WORKSPACE / "diagnostics"
DEFAULT_RUNS_ROOT = BRIDGE_DIAGNOSTICS / "being_phase_space"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Compare multiple named being phase-space watcher bundles."
    )
    parser.add_argument(
        "--runs-root",
        type=Path,
        default=DEFAULT_RUNS_ROOT,
        help="Directory containing timestamped being phase-space watcher bundles.",
    )
    parser.add_argument(
        "--bundle",
        action="append",
        type=Path,
        default=[],
        help="Explicit bundle directory to include. Can be passed more than once.",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=6,
        help="How many recent bundles to include when scanning a runs root.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help="Output directory. Defaults to workspace/diagnostics/being_phase_space_compare/<timestamp>.",
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
        return Path(target).resolve().as_posix()


def build_default_output_dir() -> Path:
    stamp = datetime.now().strftime("%Y%m%dT%H%M%S")
    return BRIDGE_DIAGNOSTICS / "being_phase_space_compare" / stamp


def discover_bundles(runs_root: Path, explicit_bundles: list[Path], limit: int) -> list[Path]:
    if explicit_bundles:
        return [path for path in explicit_bundles if (path / "summary.json").exists()]
    if not runs_root.exists():
        return []
    bundles = [
        path
        for path in runs_root.iterdir()
        if path.is_dir() and (path / "summary.json").exists()
    ]
    bundles.sort(key=lambda path: path.name)
    if limit > 0:
        bundles = bundles[-limit:]
    return bundles


def derive_window(bundle_dir: Path) -> dict[str, Any]:
    summary = load_json(bundle_dir / "summary.json")
    minime = dict(summary.get("minime") or {})
    astrid = dict(summary.get("astrid") or {})
    fill = safe_float(minime.get("current_fill_pct"))
    target = safe_float(minime.get("target_fill_pct"), 55.0)
    return {
        "bundle_dir": str(bundle_dir),
        "generated_at": summary.get("generated_at") or bundle_dir.name,
        "window_label": summary.get("window_label") or bundle_dir.name,
        "note": summary.get("note"),
        "capture_seconds": safe_float(summary.get("capture_seconds"), 0.0),
        "minime_fill_pct": fill,
        "minime_target_fill_pct": target,
        "minime_fill_delta": fill - target,
        "minime_regime": minime.get("regime"),
        "minime_pc1": safe_float((minime.get("phase_space_explained_variance") or [0.0])[0]),
        "minime_quadrant": minime.get("dominant_quadrant"),
        "minime_mean_radius": safe_float(minime.get("mean_radius")),
        "astrid_pc1": safe_float((astrid.get("phase_space_explained_variance") or [0.0])[0]),
        "minime_focus_variant": minime.get("focus_variant"),
        "best_stability_variant": dict(minime.get("best_stability") or {}).get("variant"),
        "best_openness_variant": dict(minime.get("best_openness") or {}).get("variant"),
        "astrid_input_count": int(astrid.get("input_count") or 0),
        "astrid_memory_tail_size": int(astrid.get("memory_tail_size") or 0),
        "shared_read": list(summary.get("shared_read") or []),
        "report_path": str(bundle_dir / "report.md"),
        "minime_report_path": str(bundle_dir / "minime" / "report.md"),
        "astrid_report_path": str(bundle_dir / "astrid" / "report.md"),
    }


def comparative_notes(windows: list[dict[str, Any]]) -> list[str]:
    if not windows:
        return ["No watcher bundles were found yet, so there is nothing to compare."]

    by_abs_fill = min(windows, key=lambda row: abs(safe_float(row["minime_fill_delta"])))
    max_minime_axis = max(windows, key=lambda row: safe_float(row["minime_pc1"]))
    max_astrid_axis = max(windows, key=lambda row: safe_float(row["astrid_pc1"]))
    highest_fill = max(windows, key=lambda row: safe_float(row["minime_fill_pct"]))
    lowest_fill = min(windows, key=lambda row: safe_float(row["minime_fill_pct"]))

    notes = [
        f"Closest Minime window to target fill: `{by_abs_fill['window_label']}` ({by_abs_fill['minime_fill_pct']:.2f}% on {by_abs_fill['minime_target_fill_pct']:.2f}%).",
        f"Most axis-dominated Minime regulator window: `{max_minime_axis['window_label']}` (PC1 `{max_minime_axis['minime_pc1']:.3f}`).",
        f"Most axis-dominated Astrid thematic window: `{max_astrid_axis['window_label']}` (PC1 `{max_astrid_axis['astrid_pc1']:.3f}`).",
        f"Highest-fill Minime window in this set: `{highest_fill['window_label']}` ({highest_fill['minime_fill_pct']:.2f}%).",
        f"Lowest-fill Minime window in this set: `{lowest_fill['window_label']}` ({lowest_fill['minime_fill_pct']:.2f}%).",
    ]
    if len(windows) >= 2:
        spread = safe_float(highest_fill["minime_fill_pct"]) - safe_float(lowest_fill["minime_fill_pct"])
        notes.append(
            f"Current bundle set spans `{spread:.2f}` fill points across Minime windows, which is enough to start comparing recovery-biased versus pressure-biased regulator shape."
        )
    return notes


def write_csv(output_dir: Path, windows: list[dict[str, Any]]) -> None:
    fieldnames = [
        "generated_at",
        "window_label",
        "capture_seconds",
        "minime_fill_pct",
        "minime_target_fill_pct",
        "minime_fill_delta",
        "minime_regime",
        "minime_pc1",
        "minime_quadrant",
        "minime_mean_radius",
        "astrid_pc1",
        "minime_focus_variant",
        "best_stability_variant",
        "best_openness_variant",
        "astrid_input_count",
        "astrid_memory_tail_size",
        "bundle_dir",
    ]
    with (output_dir / "windows.csv").open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fieldnames)
        writer.writeheader()
        for row in windows:
            writer.writerow({name: row.get(name) for name in fieldnames})


def write_summary(output_dir: Path, windows: list[dict[str, Any]]) -> dict[str, Any]:
    summary = {
        "generated_at": datetime.now().isoformat(),
        "window_count": len(windows),
        "notes": comparative_notes(windows),
        "windows": windows,
    }
    (output_dir / "summary.json").write_text(json.dumps(summary, indent=2))
    return summary


def write_report(output_dir: Path, summary: dict[str, Any]) -> None:
    report = [
        "# Being Phase-Space Window Comparison",
        "",
        f"Generated: `{summary['generated_at']}`",
        f"Windows compared: `{summary['window_count']}`",
        "",
        "## Comparative Read",
        "",
    ]
    report.extend(f"- {line}" for line in summary["notes"])
    report.extend(
        [
            "",
            "## Window Table",
            "",
            "| label | fill | target | delta | quadrant | mean radius | Minime PC1 | Astrid PC1 | focus | stability | openness |",
            "| --- | ---: | ---: | ---: | --- | ---: | ---: | ---: | --- | --- | --- |",
        ]
    )
    for row in summary["windows"]:
        report.append(
            "| {label} | {fill:.2f}% | {target:.2f}% | {delta:+.2f} | `{quadrant}` | {radius:.3f} | {minime_pc1:.3f} | {astrid_pc1:.3f} | `{focus}` | `{stability}` | `{openness}` |".format(
                label=row["window_label"],
                fill=safe_float(row["minime_fill_pct"]),
                target=safe_float(row["minime_target_fill_pct"]),
                delta=safe_float(row["minime_fill_delta"]),
                quadrant=row.get("minime_quadrant") or "n/a",
                radius=safe_float(row.get("minime_mean_radius")),
                minime_pc1=safe_float(row["minime_pc1"]),
                astrid_pc1=safe_float(row["astrid_pc1"]),
                focus=row.get("minime_focus_variant") or "n/a",
                stability=row.get("best_stability_variant") or "n/a",
                openness=row.get("best_openness_variant") or "n/a",
            )
        )
    report.extend(["", "## Bundles", ""])
    for row in summary["windows"]:
        bundle_dir = Path(row["bundle_dir"])
        report.append(f"### `{row['window_label']}`")
        report.append("")
        report.append(f"- Generated: `{row['generated_at']}`")
        if row.get("note"):
            report.append(f"- Note: {row['note']}")
        report.append(f"- Bundle: [{bundle_dir.name}]({safe_relpath(bundle_dir, output_dir)})")
        report.append(
            f"- Combined report: [report.md]({safe_relpath(Path(row['report_path']), output_dir)})"
        )
        report.append(
            f"- Minime report: [minime/report.md]({safe_relpath(Path(row['minime_report_path']), output_dir)})"
        )
        report.append(
            f"- Astrid report: [astrid/report.md]({safe_relpath(Path(row['astrid_report_path']), output_dir)})"
        )
        for line in row.get("shared_read") or []:
            report.append(f"- Shared read: {line}")
        report.append("")
    (output_dir / "report.md").write_text("\n".join(report) + "\n")


def main() -> int:
    args = parse_args()
    output_dir = args.output_dir or build_default_output_dir()
    ensure_dir(output_dir)

    bundles = discover_bundles(args.runs_root, args.bundle, args.limit)
    windows = [derive_window(bundle) for bundle in bundles]
    windows.sort(key=lambda row: row["generated_at"])

    summary = write_summary(output_dir, windows)
    write_csv(output_dir, windows)
    write_report(output_dir, summary)

    print(f"wrote {output_dir / 'summary.json'}")
    print(f"wrote {output_dir / 'windows.csv'}")
    print(f"wrote {output_dir / 'report.md'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
