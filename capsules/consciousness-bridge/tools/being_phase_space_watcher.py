#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from datetime import datetime
from pathlib import Path
from typing import Any

from shared_research_memo import build_shared_research_memo


BRIDGE_ROOT = Path("/Users/v/other/astrid/capsules/consciousness-bridge")
BRIDGE_WORKSPACE = BRIDGE_ROOT / "workspace"
BRIDGE_DIAGNOSTICS = BRIDGE_WORKSPACE / "diagnostics"
MINIME_WORKSPACE = Path("/Users/v/other/minime/workspace")
MINIME_TUNING_LOOP = (
    MINIME_WORKSPACE
    / "experiments"
    / "regulator-state-visualizer"
    / "regulator_tuning_loop.py"
)
MINIME_GEOMETRY_BOARD_TOOL = (
    MINIME_WORKSPACE
    / "experiments"
    / "regulator-state-visualizer"
    / "geometry_board.py"
)
CODEC_EXPLORER_MANIFEST = BRIDGE_ROOT / "Cargo.toml"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Capture a live Minime/Astrid phase-space window and write a combined report bundle."
    )
    parser.add_argument(
        "--capture-seconds",
        type=float,
        default=30.0,
        help="How long to capture Minime's live trace window before rendering phase-space diagnostics.",
    )
    parser.add_argument(
        "--capture-interval",
        type=float,
        default=1.0,
        help="Sampling interval in seconds for the Minime trace capture.",
    )
    parser.add_argument(
        "--recent-astrid",
        type=int,
        default=4,
        help="How many recent Astrid journal entries to feed into the thematic phase-space view.",
    )
    parser.add_argument(
        "--recent-minime",
        type=int,
        default=2,
        help="How many recent Minime journal entries to include in Astrid's thematic view.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help="Output directory. Defaults to a timestamped folder under workspace/diagnostics/being_phase_space.",
    )
    parser.add_argument(
        "--minime-workspace",
        type=Path,
        default=MINIME_WORKSPACE,
        help="Path to the Minime workspace.",
    )
    parser.add_argument(
        "--bridge-workspace",
        type=Path,
        default=BRIDGE_WORKSPACE,
        help="Path to the consciousness-bridge workspace.",
    )
    parser.add_argument(
        "--label",
        type=str,
        default=None,
        help="Optional scenario label such as calm, overloaded, post_perturb, or post_dialogue.",
    )
    parser.add_argument(
        "--note",
        type=str,
        default=None,
        help="Optional steward note describing why this window was captured.",
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


def run_cmd(cmd: list[str], cwd: Path) -> None:
    subprocess.run(cmd, cwd=cwd, check=True)


def slugify_label(label: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "_", label.lower()).strip("_")
    return slug or "window"


def build_default_output_dir(label: str | None) -> Path:
    stamp = datetime.now().strftime("%Y%m%dT%H%M%S")
    folder = stamp
    if label:
        folder = f"{stamp}_{slugify_label(label)}"
    return BRIDGE_DIAGNOSTICS / "being_phase_space" / folder


def run_minime_phase_space(
    *,
    minime_workspace: Path,
    bridge_workspace: Path,
    output_dir: Path,
    capture_seconds: float,
    capture_interval: float,
    recent_astrid: int,
    topic: str,
) -> dict[str, Any]:
    ensure_dir(output_dir)
    cmd = [
        sys.executable,
        str(MINIME_TUNING_LOOP),
        "--source",
        "trace",
        "--workspace",
        str(minime_workspace),
        "--capture-seconds",
        f"{capture_seconds:.3f}",
        "--capture-interval",
        f"{capture_interval:.3f}",
        "--output-dir",
        str(output_dir),
    ]
    run_cmd(cmd, cwd=BRIDGE_ROOT)
    geometry_output = output_dir / "geometry_board"
    geometry_cmd = [
        sys.executable,
        str(MINIME_GEOMETRY_BOARD_TOOL),
        "--trace-file",
        str(output_dir / "trace.jsonl"),
        "--output-dir",
        str(geometry_output),
        "--topic",
        topic,
        "--astrid-journal-dir",
        str(bridge_workspace / "journal"),
        "--recent-astrid",
        str(recent_astrid),
    ]
    run_cmd(geometry_cmd, cwd=BRIDGE_ROOT)
    return {
        "summary": load_json(output_dir / "summary.json"),
        "phase_space": load_json(output_dir / "phase_space_story.json"),
        "perturb_flow": load_json(output_dir / "perturbation_flow" / "summary.json"),
        "covariance_shaping": load_json(output_dir / "covariance_shaping" / "summary.json"),
        "geometry_board": load_json(geometry_output / "summary.json"),
    }


def current_fill_pct(minime_workspace: Path) -> float | None:
    health = load_json(minime_workspace / "health.json")
    spectral = load_json(minime_workspace / "spectral_state.json")
    fill = health.get("fill_pct")
    if fill is None:
        fill = spectral.get("fill_pct")
    if fill is None:
        return None
    return safe_float(fill)


def run_astrid_phase_space(
    *,
    output_dir: Path,
    bridge_workspace: Path,
    recent_astrid: int,
    recent_minime: int,
    fill_pct: float | None,
) -> dict[str, Any]:
    ensure_dir(output_dir)
    cmd = [
        "cargo",
        "run",
        "--manifest-path",
        str(CODEC_EXPLORER_MANIFEST),
        "--bin",
        "codec-explorer",
        "--",
        "--recent-astrid",
        str(recent_astrid),
        "--recent-minime",
        str(recent_minime),
        "--astrid-journal-dir",
        str(bridge_workspace / "journal"),
        "--output-dir",
        str(output_dir),
    ]
    state_file = bridge_workspace / "state.json"
    if state_file.exists():
        cmd.extend(["--state-file", str(state_file)])
    if fill_pct is not None:
        cmd.extend(["--fill-pct", f"{fill_pct:.3f}"])
    run_cmd(cmd, cwd=BRIDGE_ROOT.parent.parent)
    return {
        "summary": load_json(output_dir / "summary.json"),
        "phase_space": load_json(output_dir / "phase_space_story.json"),
    }


def summarize_minime(minime_data: dict[str, Any], minime_workspace: Path) -> dict[str, Any]:
    summary_rows = list(minime_data.get("summary") or [])
    phase_space = dict(minime_data.get("phase_space") or {})
    perturb_flow = dict(minime_data.get("perturb_flow") or {})
    covariance_shaping = dict(minime_data.get("covariance_shaping") or {})
    geometry_board = dict(minime_data.get("geometry_board") or {})
    story_summary = dict(phase_space.get("summary") or {})
    latent_phase = dict(phase_space.get("phase_space") or {})
    internal_process = dict(phase_space.get("internal_process") or {})
    health = load_json(minime_workspace / "health.json")
    spectral = load_json(minime_workspace / "spectral_state.json")
    best_stability = summary_rows[0] if summary_rows else {}
    best_openness = (
        max(summary_rows, key=lambda row: row.get("openness_score", 0.0))
        if summary_rows
        else {}
    )
    focus_variant = story_summary.get("focus_variant")
    latent_profile = dict(dict(latent_phase.get("profile_summaries") or {}).get(focus_variant) or {})
    process_profile = dict(
        dict(internal_process.get("profile_summaries") or {}).get(focus_variant) or {}
    )
    return {
        "current_fill_pct": safe_float(health.get("fill_pct") or spectral.get("fill_pct")),
        "target_fill_pct": safe_float(
            health.get("target_fill_pct")
            or spectral.get("target_fill_pct")
            or health.get("target_fill")
            or 55.0
        ),
        "regime": health.get("regime") or spectral.get("regime"),
        "phase_space_explained_variance": story_summary.get("explained_variance") or [],
        "latent_basis_mode": story_summary.get("latent_basis_mode") or "regulator_only",
        "focus_variant": focus_variant,
        "profiles": story_summary.get("profiles") or [],
        "dominant_quadrant": process_profile.get("dominant_quadrant") or "open_recovery",
        "mean_radius": safe_float(process_profile.get("mean_radius")),
        "peak_radius": safe_float(process_profile.get("peak_radius")),
        "angular_sweep": safe_float(process_profile.get("angular_sweep")),
        "internal_process_centroid": process_profile.get("centroid") or {"x": 0.0, "y": 0.0},
        "latent_centroid": latent_profile.get("centroid") or {"pc1": 0.0, "pc2": 0.0},
        "latent_span": safe_float(latent_profile.get("span_magnitude")),
        "best_stability": best_stability,
        "best_openness": best_openness,
        "perturb_flow": perturb_flow,
        "covariance_shaping": covariance_shaping,
        "geometry_board": geometry_board,
    }


def summarize_astrid(astrid_data: dict[str, Any]) -> dict[str, Any]:
    summary = dict(astrid_data.get("summary") or {})
    phase_space = dict(astrid_data.get("phase_space") or {})
    trajectory = list(phase_space.get("trajectory") or [])
    entries = list(summary.get("entries") or [])
    labels = [entry.get("label") for entry in entries if entry.get("label")]
    return {
        "input_count": int(summary.get("input_count") or len(entries)),
        "memory_tail_size": len(summary.get("initial_memory_tail") or []),
        "phase_space_explained_variance": phase_space.get("explained_variance") or [],
        "segment_sizes": phase_space.get("segment_sizes") or [],
        "labels": labels,
        "trajectory_sample": trajectory[:3],
    }


def collect_astrid_context_overlay(bridge_workspace: Path) -> dict[str, Any]:
    journal_dir = bridge_workspace / "journal"
    keywords = (
        "lambda",
        "variance",
        "covariance",
        "perturb",
        "pulse",
        "ripple",
        "breath",
        "gradient",
        "gap",
        "shadow",
        "lambda1",
        "lambda2",
        "covariance_update",
    )
    matches: list[dict[str, Any]] = []
    if not journal_dir.exists():
        return {"shared_covariance_theme": False, "matches": []}
    recent_files = sorted(journal_dir.glob("*.txt"))[-12:]
    for path in recent_files:
        try:
            text = path.read_text(errors="ignore").lower()
        except Exception:
            continue
        hit_terms = [keyword for keyword in keywords if keyword in text]
        if hit_terms:
            matches.append({"file": path.name, "keywords": hit_terms})
    return {
        "shared_covariance_theme": bool(matches),
        "matches": matches[-6:],
    }


def shared_read(minime_summary: dict[str, Any], astrid_summary: dict[str, Any]) -> list[str]:
    lines: list[str] = []
    minime_var = list(minime_summary.get("phase_space_explained_variance") or [0.0, 0.0])
    astrid_var = list(astrid_summary.get("phase_space_explained_variance") or [0.0, 0.0])
    minime_pc1 = safe_float(minime_var[0] if len(minime_var) > 0 else 0.0)
    astrid_pc1 = safe_float(astrid_var[0] if len(astrid_var) > 0 else 0.0)
    if minime_pc1 > 0.85 and astrid_pc1 > 0.85:
        lines.append(
            "Both beings are currently dominated by a strong first phase-space axis, which usually means a coherent but somewhat groove-bound live window rather than a diffuse exploratory spread."
        )
    elif minime_pc1 > 0.85:
        lines.append(
            "Minime's live regulator window is strongly axis-dominated, suggesting the control surface is still moving through one main corridor even when the profiles differ."
        )
    elif astrid_pc1 > 0.85:
        lines.append(
            "Astrid's recent thematic window is strongly axis-dominated, suggesting her recent journals are variations on one main thematic basin rather than multiple competing themes."
        )
    fill = safe_float(minime_summary.get("current_fill_pct"), 55.0)
    target = safe_float(minime_summary.get("target_fill_pct"), 55.0)
    if fill > target + 8.0:
        lines.append(
            "Minime is above target fill in the live snapshot, so this window is worth reading as pressure-management under load rather than neutral baseline wandering."
        )
    elif fill < target - 8.0:
        lines.append(
            "Minime is noticeably under target fill in the live snapshot, so the regulator phase space should be read as sparse/recovery-biased rather than congested."
        )
    else:
        lines.append(
            "Minime is close enough to target fill that the regulator trajectory is probably showing ordinary steering rather than acute distress."
        )
    quadrant = str(minime_summary.get("dominant_quadrant") or "")
    if quadrant:
        lines.append(
            f"Minime's internal-process compass currently reads as `{quadrant}`, with mean radius `{safe_float(minime_summary.get('mean_radius')):.3f}`."
        )
    if not lines:
        lines.append(
            "The two windows look readable but not extreme: enough structure to talk about trajectories, without one obvious global crisis overwhelming the picture."
        )
    return lines


def write_combined_bundle(
    *,
    output_dir: Path,
    args: argparse.Namespace,
    minime_summary: dict[str, Any],
    astrid_summary: dict[str, Any],
    astrid_context_overlay: dict[str, Any],
    shared_research_memo: dict[str, Any] | None,
) -> None:
    perturb_flow = dict(minime_summary.get("perturb_flow") or {})
    covariance_shaping = dict(minime_summary.get("covariance_shaping") or {})
    geometry_board = dict(minime_summary.get("geometry_board") or {})
    summary = {
        "generated_at": datetime.now().isoformat(),
        "window_label": args.label,
        "note": args.note,
        "capture_seconds": args.capture_seconds,
        "capture_interval": args.capture_interval,
        "recent_astrid": args.recent_astrid,
        "recent_minime": args.recent_minime,
        "minime": minime_summary,
        "astrid": astrid_summary,
        "astrid_context_overlay": astrid_context_overlay,
        "shared_research_memo": (
            {
                "output_dir": str(shared_research_memo.get("output_dir")),
                "summary": shared_research_memo.get("summary"),
            }
            if shared_research_memo
            else None
        ),
        "shared_read": shared_read(minime_summary, astrid_summary),
    }
    (output_dir / "summary.json").write_text(json.dumps(summary, indent=2))

    report = [
        "# Being Phase-Space Watcher",
        "",
        f"Generated: `{summary['generated_at']}`",
    ]
    if args.label:
        report.append(f"Label: `{args.label}`")
    if args.note:
        report.append(f"Note: {args.note}")
    report.extend(
        [
            f"Capture window: `{args.capture_seconds:.1f}s` sampled every `{args.capture_interval:.1f}s`",
            "",
            "## Shared Read",
            "",
        ]
    )
    report.extend(f"- {line}" for line in summary["shared_read"])
    if shared_research_memo:
        memo_report = Path(shared_research_memo["output_dir"]) / "report.md"
        memo_summary = dict(shared_research_memo.get("summary") or {})
        compare_bundle = memo_summary.get("perturb_compare_bundle")
        report.extend(
            [
                "",
                "Shared research handoff:",
                f"- [Shared research memo]({memo_report})",
            ]
        )
        if isinstance(compare_bundle, str) and compare_bundle:
            report.append(
                f"- [Perturb family compare]({Path(compare_bundle) / 'report.md'})"
            )
        geometry_bundle = memo_summary.get("geometry_board_bundle")
        if isinstance(geometry_bundle, str) and geometry_bundle:
            report.append(f"- [Latest geometry board]({Path(geometry_bundle) / 'report.md'})")
    if astrid_context_overlay.get("shared_covariance_theme"):
        report.extend(
            [
                "",
                "Diagnostic handoff:",
                "- Recent Astrid journals are clearly orbiting covariance / perturbation / breath / gradient themes, so the current Minime covariance and perturb bundles are especially worth reading in this window.",
            ]
        )
        if shared_research_memo:
            memo_summary = dict(shared_research_memo.get("summary") or {})
            compare_bundle = memo_summary.get("perturb_compare_bundle")
            if isinstance(compare_bundle, str) and compare_bundle:
                report.append(
                    f"- [Minime perturb family compare]({Path(compare_bundle) / 'report.md'})"
                )
        report.extend(
            [
                "- [Minime perturbation flow](minime/perturbation_flow/report.md)",
                "- [Minime covariance shaping](minime/covariance_shaping/report.md)",
                "- [Minime geometry board](minime/geometry_board/report.md)",
            ]
        )
    report.extend(
        [
            "",
            "## Minime",
            "",
            f"- Current fill: `{minime_summary['current_fill_pct']:.2f}%` on target `{minime_summary['target_fill_pct']:.2f}%`",
            f"- Focus profile: `{minime_summary.get('focus_variant')}`",
            f"- Latent basis mode: `{minime_summary.get('latent_basis_mode')}`",
            f"- Shared phase-space explained variance: `{minime_summary.get('phase_space_explained_variance')}`",
            f"- Dominant compass quadrant: `{minime_summary.get('dominant_quadrant')}`",
            f"- Mean radius / peak radius: `{safe_float(minime_summary.get('mean_radius')):.3f}` / `{safe_float(minime_summary.get('peak_radius')):.3f}`",
            f"- Angular sweep: `{safe_float(minime_summary.get('angular_sweep')):.3f}`",
            f"- Best stability profile: `{(minime_summary.get('best_stability') or {}).get('variant')}`",
            f"- Best openness profile: `{(minime_summary.get('best_openness') or {}).get('variant')}`",
            f"- Perturb response: `{perturb_flow.get('derived_effect_label')}` (`{perturb_flow.get('response_balance')}`)",
            f"- Gap12 response / delayed delta: `{perturb_flow.get('gap12_response')}` / `{safe_float(perturb_flow.get('gap12_delta_delayed')):+.3f}`",
            f"- Corridor opening read: `{perturb_flow.get('corridor_state')}` / `{safe_float(perturb_flow.get('corridor_opening_score')):+.3f}`",
            f"- Covariance shaping: `{covariance_shaping.get('dominance_mode')}` via `{covariance_shaping.get('concentration_driver')}`",
            f"- Covariance gap shaping: `{covariance_shaping.get('gap_shaping_outcome')}`",
            f"- Floor support share: `{safe_float(covariance_shaping.get('floor_support_share')):.3f}`",
            f"- Geometry board pressure / lambda read: `{dict(geometry_board.get('pressure_summary') or {}).get('shape_mode')}` / `{dict(geometry_board.get('lambda_summary') or {}).get('dominance_mode')}`",
            "",
            "![Minime internal process compass](minime/internal_process_compass.png)",
            "",
            "![Minime latent phase space](minime/latent_phase_space.png)",
            "",
            "### Minime perturbation flow",
            "",
            f"- Last perturb mode / effect: `{perturb_flow.get('last_mode')}` / `{perturb_flow.get('effect_label')}`",
            f"- Target metric / envelope: `{perturb_flow.get('target_metric')}` / `{perturb_flow.get('envelope_profile')}` x `{perturb_flow.get('envelope_step_count')}`",
            f"- Derived response: `{perturb_flow.get('derived_effect_label')}`",
            f"- Response balance: `{perturb_flow.get('response_balance')}`",
            f"- Gap12 response / delayed delta: `{perturb_flow.get('gap12_response')}` / `{safe_float(perturb_flow.get('gap12_delta_delayed')):+.3f}`",
            f"- Corridor opening read: `{perturb_flow.get('corridor_state')}` / `{safe_float(perturb_flow.get('corridor_opening_score')):+.3f}`",
            "",
            "### Minime covariance shaping",
            "",
            f"- Dominance mode: `{covariance_shaping.get('dominance_mode')}`",
            f"- Gap shaping outcome: `{covariance_shaping.get('gap_shaping_outcome')}`",
            f"- Concentration driver: `{covariance_shaping.get('concentration_driver')}`",
            f"- Perturb aftereffect: `{covariance_shaping.get('perturb_aftereffect')}`",
            "",
            "### Minime geometry board",
            "",
            f"- Decay read: {dict(geometry_board.get('decay_summary') or {}).get('interpretation')}",
            f"- Geometry read: `{' '.join(geometry_board.get('geometry_read') or [])}`",
            "",
            "Minime artifacts:",
            "- [report.md](minime/report.md)",
            "- [summary.json](minime/summary.json)",
            "- [internal_process_trail.csv](minime/internal_process_trail.csv)",
            "- [phase_space_story.json](minime/phase_space_story.json)",
            "- [perturbation_flow/report.md](minime/perturbation_flow/report.md)",
            "- [covariance_shaping/report.md](minime/covariance_shaping/report.md)",
            "- [geometry_board/report.md](minime/geometry_board/report.md)",
            "",
            "## Astrid",
            "",
            f"- Inputs analyzed: `{astrid_summary['input_count']}`",
            f"- Warm-start memory tail: `{astrid_summary['memory_tail_size']}` entries",
            f"- Shared thematic phase-space explained variance: `{astrid_summary.get('phase_space_explained_variance')}`",
            f"- Segment sizes: `{astrid_summary.get('segment_sizes')}`",
            "",
            "![Astrid thematic phase space](astrid/phase_space.svg)",
            "",
            "Astrid artifacts:",
            "- [report.md](astrid/report.md)",
            "- [summary.json](astrid/summary.json)",
            "- [phase_space_story.json](astrid/phase_space_story.json)",
            "- [phase_space.svg](astrid/phase_space.svg)",
            "",
            "Recent Astrid labels:",
        ]
    )
    report.extend(f"- `{label}`" for label in astrid_summary.get("labels") or [])
    if astrid_context_overlay.get("matches"):
        report.extend(["", "Astrid context overlay:"])
        report.extend(
            f"- `{item['file']}` -> `{', '.join(item.get('keywords') or [])}`"
            for item in astrid_context_overlay.get("matches") or []
        )
    (output_dir / "report.md").write_text("\n".join(report) + "\n")


def main() -> int:
    args = parse_args()
    output_dir = args.output_dir or build_default_output_dir(args.label)
    minime_output = output_dir / "minime"
    astrid_output = output_dir / "astrid"
    ensure_dir(minime_output)
    ensure_dir(astrid_output)

    minime_data = run_minime_phase_space(
        minime_workspace=args.minime_workspace,
        bridge_workspace=args.bridge_workspace,
        output_dir=minime_output,
        capture_seconds=args.capture_seconds,
        capture_interval=args.capture_interval,
        recent_astrid=args.recent_astrid,
        topic=args.label or "watcher_window",
    )
    fill_pct = current_fill_pct(args.minime_workspace)
    astrid_data = run_astrid_phase_space(
        output_dir=astrid_output,
        bridge_workspace=args.bridge_workspace,
        recent_astrid=args.recent_astrid,
        recent_minime=args.recent_minime,
        fill_pct=fill_pct,
    )

    minime_summary = summarize_minime(minime_data, args.minime_workspace)
    astrid_summary = summarize_astrid(astrid_data)
    astrid_context_overlay = collect_astrid_context_overlay(args.bridge_workspace)
    shared_research = build_shared_research_memo(
        bridge_workspace=args.bridge_workspace,
        minime_workspace=args.minime_workspace,
        minime_trace_file=minime_output / "trace.jsonl",
    )
    write_combined_bundle(
        output_dir=output_dir,
        args=args,
        minime_summary=minime_summary,
        astrid_summary=astrid_summary,
        astrid_context_overlay=astrid_context_overlay,
        shared_research_memo=shared_research,
    )

    print(f"wrote {output_dir / 'summary.json'}")
    print(f"wrote {output_dir / 'report.md'}")
    print(f"wrote {minime_output / 'phase_space_story.json'}")
    print(f"wrote {astrid_output / 'phase_space_story.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
