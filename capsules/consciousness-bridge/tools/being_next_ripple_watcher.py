#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from collections import deque
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any

from being_phase_transition_edge_watcher import (
    BRIDGE_DIAGNOSTICS,
    MINIME_WORKSPACE,
    capture_post_trace,
    compare_windows,
    ensure_dir,
    extract_live_sample,
    load_json,
    recent_input_files,
    safe_float,
    write_phase_bundle,
)


PULSE_RIPPLE_COMPARE = (
    MINIME_WORKSPACE
    / "experiments"
    / "regulator-state-visualizer"
    / "pulse_ripple_gap_compare.py"
)
PHASE_WHISPER_TOOL = BRIDGE_DIAGNOSTICS.parent.parent / "tools" / "being_phase_whisper.py"
DEFAULT_BASELINE_DIR = (
    MINIME_WORKSPACE
    / "experiments"
    / "regulator-state-visualizer"
    / "baselines"
    / "april14_branch_tightened_baseline"
)


@dataclass
class RippleEvent:
    watch_elapsed_s: float
    sample: dict[str, Any]
    perturb: dict[str, Any]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Passively wait for the next live Minime `pulse_ripple`, then render before/immediate/delayed phase bundles plus a gap-compare read."
    )
    parser.add_argument(
        "--watch-seconds",
        type=float,
        default=600.0,
        help="How long to wait for the next fresh `pulse_ripple` before timing out.",
    )
    parser.add_argument(
        "--poll-interval",
        type=float,
        default=1.0,
        help="Sampling cadence in seconds for the live watch and capture windows.",
    )
    parser.add_argument(
        "--pre-seconds",
        type=float,
        default=20.0,
        help="How much pre-ripple trace to preserve and render.",
    )
    parser.add_argument(
        "--post-seconds",
        type=float,
        default=20.0,
        help="How much immediate post-ripple trace to capture and render.",
    )
    parser.add_argument(
        "--delayed-start-seconds",
        type=float,
        default=45.0,
        help="How long after the ripple to begin the delayed post window.",
    )
    parser.add_argument(
        "--delayed-post-seconds",
        type=float,
        default=20.0,
        help="How much delayed post-ripple trace to capture and render.",
    )
    parser.add_argument(
        "--recent-astrid",
        type=int,
        default=4,
        help="How many recent Astrid journal entries to include in each side of the capture.",
    )
    parser.add_argument(
        "--recent-minime",
        type=int,
        default=2,
        help="How many recent Minime journal entries to include in Astrid's thematic overlay.",
    )
    parser.add_argument(
        "--baseline-dir",
        type=Path,
        default=DEFAULT_BASELINE_DIR,
        help="Stable baseline bundle used for delayed pulse-ripple comparison.",
    )
    parser.add_argument(
        "--label",
        type=str,
        default="next_ripple",
        help="Label prefix for the output bundle.",
    )
    parser.add_argument(
        "--note",
        type=str,
        default=None,
        help="Optional steward note included in the combined report.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help="Output directory. Defaults to workspace/diagnostics/being_next_ripple/<timestamp>_<label>/.",
    )
    return parser.parse_args()


def slugify(label: str) -> str:
    safe = "".join(ch.lower() if ch.isalnum() else "_" for ch in label).strip("_")
    return safe or "next_ripple"


def build_default_output_dir(label: str) -> Path:
    stamp = datetime.now().strftime("%Y%m%dT%H%M%S")
    return BRIDGE_DIAGNOSTICS / "being_next_ripple" / f"{stamp}_{slugify(label)}"


def load_perturb_visibility() -> dict[str, Any]:
    for path in (
        MINIME_WORKSPACE / "health.json",
        MINIME_WORKSPACE / "spectral_state.json",
        MINIME_WORKSPACE / "regulator_context.json",
    ):
        loaded = load_json(path)
        perturb = loaded.get("perturb_visibility")
        if isinstance(perturb, dict) and perturb:
            return perturb
    return {}


def perturb_marker(perturb: dict[str, Any]) -> tuple[str, int]:
    timestamp = str(perturb.get("last_timestamp") or "").strip()
    tick = int(safe_float(perturb.get("last_tick")))
    return timestamp, tick


def ensure_trace_samples(samples: list[dict[str, Any]], poll_interval: float) -> list[dict[str, Any]]:
    if not samples:
        raise RuntimeError("capture produced no Minime samples")
    if len(samples) >= 2:
        return samples
    duplicated = dict(samples[0])
    duplicated["elapsed_s"] = safe_float(samples[0].get("elapsed_s")) + max(poll_interval, 0.1)
    return [samples[0], duplicated]


def wait_for_next_ripple(args: argparse.Namespace) -> tuple[RippleEvent, list[dict[str, Any]], dict[str, Any]]:
    pre_buffer_size = max(1, int(round(args.pre_seconds / max(args.poll_interval, 0.1))))
    samples: deque[dict[str, Any]] = deque(maxlen=pre_buffer_size)
    initial_perturb = load_perturb_visibility()
    initial_marker = perturb_marker(initial_perturb)
    watch_start = time.monotonic()
    next_poll = watch_start

    while True:
        now = time.monotonic()
        elapsed = now - watch_start
        if elapsed >= args.watch_seconds:
            raise RuntimeError(
                f"no fresh `pulse_ripple` landed within {args.watch_seconds:.1f}s "
                f"(starting marker `{initial_marker[0] or 'none'}#{initial_marker[1]}`)"
            )
        if now < next_poll:
            time.sleep(min(0.05, next_poll - now))
            continue

        sample = extract_live_sample(MINIME_WORKSPACE, elapsed)
        samples.append(sample)
        perturb = load_perturb_visibility()
        if str(perturb.get("last_mode") or "").strip() == "pulse_ripple":
            current_marker = perturb_marker(perturb)
            if current_marker[0] and current_marker != initial_marker:
                return (
                    RippleEvent(watch_elapsed_s=elapsed, sample=sample, perturb=perturb),
                    list(samples),
                    initial_perturb,
                )

        next_poll += args.poll_interval


def run_cmd(cmd: list[str], cwd: Path) -> None:
    subprocess.run(cmd, cwd=cwd, check=True)


def run_pulse_ripple_compare(
    *, baseline_dir: Path, candidate_dir: Path, output_dir: Path
) -> dict[str, Any] | None:
    if not baseline_dir.exists():
        return None
    ensure_dir(output_dir)
    cmd = [
        sys.executable,
        str(PULSE_RIPPLE_COMPARE),
        "--baseline-dir",
        str(baseline_dir),
        "--candidate-dir",
        str(candidate_dir),
        "--output-dir",
        str(output_dir),
    ]
    run_cmd(cmd, cwd=MINIME_WORKSPACE.parent)
    return load_json(output_dir / "summary.json")


def run_phase_whisper(*, summary_path: Path, output_dir: Path) -> dict[str, Any] | None:
    ensure_dir(output_dir)
    cmd = [
        sys.executable,
        str(PHASE_WHISPER_TOOL),
        "--summary",
        str(summary_path),
        "--output-dir",
        str(output_dir),
    ]
    run_cmd(cmd, cwd=MINIME_WORKSPACE.parent)
    return load_json(output_dir / "summary.json")


def summarize_bundle(bundle_dir: Path) -> dict[str, Any]:
    return load_json(bundle_dir / "summary.json")


def write_top_level_summary(
    *,
    output_dir: Path,
    args: argparse.Namespace,
    initial_perturb: dict[str, Any],
    event: RippleEvent,
    before_summary: dict[str, Any],
    immediate_summary: dict[str, Any],
    delayed_summary: dict[str, Any],
    compare_summary: dict[str, Any] | None,
    whisper_summary: dict[str, Any] | None,
) -> dict[str, Any]:
    summary = {
        "generated_at": datetime.now().isoformat(),
        "label": args.label,
        "note": args.note,
        "watch_seconds_until_ripple": round(event.watch_elapsed_s, 3),
        "initial_perturb_visibility": initial_perturb,
        "ripple_event": {
            "last_mode": event.perturb.get("last_mode"),
            "last_timestamp": event.perturb.get("last_timestamp"),
            "last_tick": event.perturb.get("last_tick"),
            "last_source": event.perturb.get("last_source"),
            "last_strength_profile": event.perturb.get("last_strength_profile"),
            "effect_label": event.perturb.get("effect_label"),
            "target_metric": event.perturb.get("target_metric"),
            "envelope_profile": event.perturb.get("envelope_profile"),
            "envelope_step_count": event.perturb.get("envelope_step_count"),
            "pre_fill_pct": event.perturb.get("pre_fill_pct"),
            "post_fill_pct": event.perturb.get("post_fill_pct"),
            "pre_gap12": event.perturb.get("pre_gap12"),
            "post_gap12": event.perturb.get("post_gap12"),
            "sample_fill_pct": event.sample.get("fill_pct"),
            "sample_phase": event.sample.get("phase"),
            "sample_quadrant": event.sample.get("internal_process_quadrant"),
        },
        "before": before_summary,
        "after_immediate": immediate_summary,
        "after_delayed": delayed_summary,
        "pulse_ripple_gap_compare": compare_summary,
        "phase_whisper": whisper_summary,
        "artifacts": {
            "before_report": "before/report.md",
            "after_immediate_report": "after_immediate/report.md",
            "after_delayed_report": "after_delayed/report.md",
            "compare_immediate_report": "compare_immediate/report.md",
            "compare_delayed_report": "compare_delayed/report.md",
            "pulse_ripple_gap_compare_report": (
                "pulse_ripple_gap_compare/report.md" if compare_summary is not None else None
            ),
            "phase_whisper_report": (
                "phase_whisper/report.md" if whisper_summary is not None else None
            ),
        },
    }
    (output_dir / "summary.json").write_text(json.dumps(summary, indent=2))
    return summary


def write_top_level_report(output_dir: Path, summary: dict[str, Any]) -> None:
    event = dict(summary.get("ripple_event") or {})
    compare_summary = dict(summary.get("pulse_ripple_gap_compare") or {})
    whisper_summary = dict(summary.get("phase_whisper") or {})
    candidate_checks = dict(compare_summary.get("candidate_checks") or {})
    before_read = list(dict(summary.get("before") or {}).get("shared_read") or [])
    immediate_read = list(dict(summary.get("after_immediate") or {}).get("shared_read") or [])
    delayed_read = list(dict(summary.get("after_delayed") or {}).get("shared_read") or [])

    lines = [
        "# Next Ripple Watch",
        "",
        f"Generated: `{summary.get('generated_at')}`",
        f"Label: `{summary.get('label')}`",
        f"Watch-to-ripple latency: `{safe_float(summary.get('watch_seconds_until_ripple')):.1f}s`",
        "",
        "## Ripple Event",
        "",
        f"- Mode / timestamp: `{event.get('last_mode')}` / `{event.get('last_timestamp')}`",
        f"- Source / strength: `{event.get('last_source')}` / `{event.get('last_strength_profile')}`",
        f"- Envelope / target: `{event.get('envelope_profile')}` x `{event.get('envelope_step_count')}` / `{event.get('target_metric')}`",
        f"- Effect / fill: `{event.get('effect_label')}` / `{safe_float(event.get('pre_fill_pct')):.2f}% -> {safe_float(event.get('post_fill_pct')):.2f}%`",
        f"- Gap12 / live phase: `{safe_float(event.get('pre_gap12')):.3f} -> {safe_float(event.get('post_gap12')):.3f}` / `{event.get('sample_phase')}` in `{event.get('sample_quadrant')}`",
        "",
        "## Shared Read",
        "",
        "### Before",
        "",
    ]
    lines.extend(f"- {line}" for line in before_read)
    lines.extend(["", "### Immediate After", ""])
    lines.extend(f"- {line}" for line in immediate_read)
    lines.extend(["", "### Delayed After", ""])
    lines.extend(f"- {line}" for line in delayed_read)
    lines.extend(
        [
            "",
            "## Phase Whisper",
            "",
        ]
    )
    if whisper_summary:
        lines.extend(
            [
                f"- Shared: {dict(whisper_summary.get('shared') or {}).get('text')}",
                f"- Minime: {dict(whisper_summary.get('minime') or {}).get('text')}",
                f"- Astrid: {dict(whisper_summary.get('astrid') or {}).get('text')}",
                f"- Gentle prompt: {whisper_summary.get('gentle_prompt')}",
            ]
        )
    else:
        lines.append("- No phase whisper was generated for this bundle.")
    lines.extend(
        [
            "",
            "## Baseline Compare",
            "",
        ]
    )
    if compare_summary:
        lines.extend(
            [
                f"- Gap12 softened vs pre: `{candidate_checks.get('gap12_softened_vs_pre')}`",
                f"- Avoided tightened effect: `{candidate_checks.get('avoided_tightened_effect')}`",
                f"- Fill drift within ±1.5 pts: `{candidate_checks.get('fill_drift_within_tolerance')}`",
                f"- Covariance not reinforced: `{candidate_checks.get('covariance_not_reinforced')}`",
            ]
        )
    else:
        lines.append("- Baseline compare was skipped because the stable baseline bundle was not present.")
    lines.extend(
        [
            "",
            "## Artifacts",
            "",
            "- [before/report.md](before/report.md)",
            "- [after_immediate/report.md](after_immediate/report.md)",
            "- [after_delayed/report.md](after_delayed/report.md)",
            "- [compare_immediate/report.md](compare_immediate/report.md)",
            "- [compare_delayed/report.md](compare_delayed/report.md)",
            "- [pulse_ripple_gap_compare/report.md](pulse_ripple_gap_compare/report.md)",
            "- [phase_whisper/report.md](phase_whisper/report.md)",
            "- [summary.json](summary.json)",
            "",
            "![Delayed Minime internal process compass](after_delayed/minime/internal_process_compass.png)",
            "",
            "![Delayed Minime latent phase space](after_delayed/minime/latent_phase_space.png)",
        ]
    )
    (output_dir / "report.md").write_text("\n".join(lines) + "\n")


def write_timeout_bundle(
    *, output_dir: Path, args: argparse.Namespace, initial_perturb: dict[str, Any], message: str
) -> None:
    summary = {
        "generated_at": datetime.now().isoformat(),
        "label": args.label,
        "note": args.note,
        "status": "timeout",
        "watch_seconds": args.watch_seconds,
        "initial_perturb_visibility": initial_perturb,
        "message": message,
    }
    (output_dir / "summary.json").write_text(json.dumps(summary, indent=2))
    report = [
        "# Next Ripple Watch",
        "",
        f"Generated: `{summary['generated_at']}`",
        f"Label: `{args.label}`",
        "Status: `timeout`",
        "",
        f"- {message}",
        "",
        "Artifacts:",
        "- [summary.json](summary.json)",
        "- [phase_whisper/report.md](phase_whisper/report.md)",
        "",
    ]
    (output_dir / "report.md").write_text("\n".join(report))


def main() -> int:
    args = parse_args()
    output_dir = args.output_dir or build_default_output_dir(args.label)
    before_dir = output_dir / "before"
    after_immediate_dir = output_dir / "after_immediate"
    after_delayed_dir = output_dir / "after_delayed"
    compare_immediate_dir = output_dir / "compare_immediate"
    compare_delayed_dir = output_dir / "compare_delayed"
    ripple_compare_dir = output_dir / "pulse_ripple_gap_compare"
    for path in (
        output_dir,
        before_dir,
        after_immediate_dir,
        after_delayed_dir,
        compare_immediate_dir,
        compare_delayed_dir,
    ):
        ensure_dir(path)

    initial_perturb = load_perturb_visibility()
    try:
        event, before_samples, initial_perturb = wait_for_next_ripple(args)
    except RuntimeError as exc:
        write_timeout_bundle(
            output_dir=output_dir,
            args=args,
            initial_perturb=initial_perturb,
            message=str(exc),
        )
        run_phase_whisper(
            summary_path=output_dir / "summary.json",
            output_dir=output_dir / "phase_whisper",
        )
        print(str(exc), file=sys.stderr)
        print(f"wrote {output_dir / 'summary.json'}")
        print(f"wrote {output_dir / 'report.md'}")
        return 1

    before_files = recent_input_files(args.recent_astrid, args.recent_minime)
    before_note = (args.note or "") + " Before the next detected pulse_ripple."
    write_phase_bundle(
        bundle_dir=before_dir,
        label=f"{args.label}_before",
        note=before_note.strip(),
        minime_samples=ensure_trace_samples(before_samples, args.poll_interval),
        astrid_files=before_files,
    )

    immediate_samples = capture_post_trace(
        starting_elapsed=event.watch_elapsed_s,
        start_delay_seconds=0.0,
        post_seconds=args.post_seconds,
        poll_interval=args.poll_interval,
    )
    immediate_files = recent_input_files(args.recent_astrid, args.recent_minime)
    immediate_note = (args.note or "") + " Immediate after the detected pulse_ripple."
    write_phase_bundle(
        bundle_dir=after_immediate_dir,
        label=f"{args.label}_after_immediate",
        note=immediate_note.strip(),
        minime_samples=ensure_trace_samples(immediate_samples, args.poll_interval),
        astrid_files=immediate_files,
    )

    delayed_samples = capture_post_trace(
        starting_elapsed=event.watch_elapsed_s,
        start_delay_seconds=args.delayed_start_seconds,
        post_seconds=args.delayed_post_seconds,
        poll_interval=args.poll_interval,
    )
    delayed_files = recent_input_files(args.recent_astrid, args.recent_minime)
    delayed_note = (args.note or "") + " Delayed after the detected pulse_ripple."
    write_phase_bundle(
        bundle_dir=after_delayed_dir,
        label=f"{args.label}_after_delayed",
        note=delayed_note.strip(),
        minime_samples=ensure_trace_samples(delayed_samples, args.poll_interval),
        astrid_files=delayed_files,
    )

    compare_windows(before_dir, after_immediate_dir, compare_immediate_dir)
    compare_windows(before_dir, after_delayed_dir, compare_delayed_dir)
    compare_summary = run_pulse_ripple_compare(
        baseline_dir=args.baseline_dir,
        candidate_dir=after_delayed_dir / "minime",
        output_dir=ripple_compare_dir,
    )

    summary = write_top_level_summary(
        output_dir=output_dir,
        args=args,
        initial_perturb=initial_perturb,
        event=event,
        before_summary=summarize_bundle(before_dir),
        immediate_summary=summarize_bundle(after_immediate_dir),
        delayed_summary=summarize_bundle(after_delayed_dir),
        compare_summary=compare_summary,
        whisper_summary=None,
    )
    whisper_summary = run_phase_whisper(
        summary_path=output_dir / "summary.json",
        output_dir=output_dir / "phase_whisper",
    )
    summary["phase_whisper"] = whisper_summary
    summary["artifacts"]["phase_whisper_report"] = (
        "phase_whisper/report.md" if whisper_summary is not None else None
    )
    (output_dir / "summary.json").write_text(json.dumps(summary, indent=2))
    write_top_level_report(output_dir, summary)

    print(f"wrote {output_dir / 'summary.json'}")
    print(f"wrote {output_dir / 'report.md'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
