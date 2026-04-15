#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import math
import subprocess
import sys
import time
from collections import deque
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any

from being_phase_transition_report import (
    build_transition_summary,
    shared_read,
    summarize_astrid,
    summarize_minime,
    write_transition_report,
)


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
CODEC_EXPLORER_MANIFEST = BRIDGE_ROOT / "Cargo.toml"
COMPARE_TOOL = BRIDGE_ROOT / "tools" / "being_phase_space_compare.py"
ARCHIVE_TOOL = BRIDGE_ROOT / "tools" / "being_phase_transition_archive.py"


@dataclass
class EdgeEvent:
    kind: str
    description: str
    watch_elapsed_s: float
    previous: dict[str, Any] | None
    current: dict[str, Any]
    trigger_mode: str
    confidence: str
    event_payload: dict[str, Any] | None = None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Passively watch for a live Minime transition edge, then capture before/after being phase-space bundles."
    )
    parser.add_argument(
        "--edge",
        choices=(
            "any",
            "phase_transition",
            "fill_crossing",
            "fill_band",
            "calm_flip",
            "lambda_jump",
            "spectral_spike",
            "backend_hot",
        ),
        default="any",
        help="Which live edge to watch for before capturing before/after windows.",
    )
    parser.add_argument(
        "--watch-seconds",
        type=float,
        default=180.0,
        help="How long to passively watch for an edge before timing out.",
    )
    parser.add_argument(
        "--poll-interval",
        type=float,
        default=1.0,
        help="Polling cadence for the live state watcher.",
    )
    parser.add_argument(
        "--pre-seconds",
        type=float,
        default=20.0,
        help="How much pre-edge trace to preserve and render.",
    )
    parser.add_argument(
        "--post-seconds",
        type=float,
        default=20.0,
        help="How much immediate post-edge trace to capture and render.",
    )
    parser.add_argument(
        "--delayed-start-seconds",
        type=float,
        default=45.0,
        help="How long after the edge to begin the delayed post window.",
    )
    parser.add_argument(
        "--delayed-post-seconds",
        type=float,
        default=20.0,
        help="How much delayed post-edge trace to capture and render.",
    )
    parser.add_argument(
        "--recent-astrid",
        type=int,
        default=4,
        help="How many recent Astrid journal files each side should include.",
    )
    parser.add_argument(
        "--recent-minime",
        type=int,
        default=2,
        help="How many recent Minime journal files each side should include.",
    )
    parser.add_argument(
        "--fill-band-threshold",
        type=float,
        default=6.0,
        help="Distance from target fill used to classify under/near/over bands.",
    )
    parser.add_argument(
        "--lambda-jump-threshold",
        type=float,
        default=0.08,
        help="Minimum instantaneous lambda_stress jump to count as an edge.",
    )
    parser.add_argument(
        "--lambda-stress-floor",
        type=float,
        default=0.12,
        help="Minimum resulting lambda_stress to count as a lambda jump edge.",
    )
    parser.add_argument(
        "--allow-current-state-trigger",
        action="store_true",
        help="Allow the watcher to trigger once the pre-buffer is full if the current state already matches the selected edge.",
    )
    parser.add_argument(
        "--current-state-confirmations",
        type=int,
        default=2,
        help="How many consecutive current-state matches are required before a low-confidence trigger fires.",
    )
    parser.add_argument(
        "--label",
        type=str,
        default="transition_edge",
        help="Label prefix for the capture bundle.",
    )
    parser.add_argument(
        "--note",
        type=str,
        default=None,
        help="Optional steward note to include in the bundle.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help="Output directory. Defaults to workspace/diagnostics/being_phase_transition_edges/<timestamp>_<label>/.",
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


def safe_bool(value: Any) -> bool:
    return bool(value)


def run_cmd(cmd: list[str], cwd: Path) -> None:
    subprocess.run(cmd, cwd=cwd, check=True)


def build_default_output_dir(label: str) -> Path:
    stamp = datetime.now().strftime("%Y%m%dT%H%M%S")
    safe = "".join(ch.lower() if ch.isalnum() else "_" for ch in label).strip("_") or "transition_edge"
    return BRIDGE_DIAGNOSTICS / "being_phase_transition_edges" / f"{stamp}_{safe}"


def recent_files(dir_path: Path, limit: int) -> list[Path]:
    if limit <= 0 or not dir_path.exists():
        return []
    entries: list[tuple[float, Path]] = []
    for path in dir_path.iterdir():
        if not path.is_file():
            continue
        if path.suffix.lower() not in {".txt", ".md"}:
            continue
        try:
            modified = path.stat().st_mtime
        except OSError:
            continue
        entries.append((modified, path))
    entries.sort(key=lambda item: item[0], reverse=True)
    return [path for _, path in entries[:limit]]


def extract_live_sample(workspace: Path, elapsed_s: float) -> dict[str, Any]:
    health = load_json(workspace / "health.json")
    spectral = load_json(workspace / "spectral_state.json")
    regulator = load_json(workspace / "regulator_context.json")
    sovereignty = load_json(workspace / "sovereignty_state.json")
    pi = health.get("pi") or {}
    llm_backend = health.get("llm_backend_health") or spectral.get("llm_backend_health") or {}
    return {
        "elapsed_s": round(elapsed_s, 3),
        "fill_pct": safe_float(health.get("fill_pct") or spectral.get("fill_pct")),
        "lambda1_rel": safe_float(health.get("lambda1_rel") or spectral.get("lambda1_rel"), 1.0),
        "lambda_deviation": safe_float(
            health.get("lambda_deviation") or spectral.get("lambda_deviation")
        ),
        "lambda_stress": safe_float(health.get("lambda_stress") or spectral.get("lambda_stress")),
        "geom_rel": safe_float(health.get("geom_rel") or spectral.get("geom_rel"), 1.0),
        "geom_drive_raw": safe_float(
            health.get("geom_drive_raw") or spectral.get("geom_drive_raw")
        ),
        "geom_drive_effective": safe_float(
            health.get("geom_drive_effective") or spectral.get("geom_drive_effective")
        ),
        "target_fill": safe_float(
            spectral.get("target_fill")
            or spectral.get("target_fill_pct")
            or health.get("target_fill_pct")
            or pi.get("target_fill")
            or regulator.get("adaptive_target"),
            55.0,
        ),
        "target_lambda1_rel": safe_float(
            spectral.get("target_lambda1_rel") or pi.get("target_lambda1_rel"),
            1.05,
        ),
        "target_geom_rel": safe_float(
            spectral.get("target_geom_rel") or regulator.get("target_geom_rel"),
            1.0,
        ),
        "spectral_entropy": safe_float(spectral.get("spectral_entropy")),
        "structural_entropy": safe_float(spectral.get("structural_entropy")),
        "spectral_glimpse_12d": spectral.get("spectral_glimpse_12d")
        or health.get("spectral_glimpse_12d")
        or regulator.get("spectral_glimpse_12d"),
        "internal_process_x": safe_float(
            spectral.get("internal_process_x")
            or health.get("internal_process_x")
            or regulator.get("internal_process_x")
        ),
        "internal_process_y": safe_float(
            spectral.get("internal_process_y")
            or health.get("internal_process_y")
            or regulator.get("internal_process_y")
        ),
        "internal_process_radius": safe_float(
            spectral.get("internal_process_radius")
            or health.get("internal_process_radius")
            or regulator.get("internal_process_radius")
        ),
        "internal_process_theta": safe_float(
            spectral.get("internal_process_theta")
            or health.get("internal_process_theta")
            or regulator.get("internal_process_theta")
        ),
        "internal_process_quadrant": spectral.get("internal_process_quadrant")
        or health.get("internal_process_quadrant")
        or regulator.get("internal_process_quadrant")
        or "open_recovery",
        "gate": safe_float(health.get("gate") or spectral.get("gate"), 1.0),
        "gate_raw": safe_float(health.get("gate_raw") or spectral.get("gate_raw")),
        "filt": safe_float(health.get("filt") or spectral.get("filt")),
        "filt_raw": safe_float(health.get("filt_raw") or spectral.get("filt_raw")),
        "deadband_fill": safe_float(spectral.get("deadband_fill") or pi.get("deadband_fill")),
        "intrinsic_wander": safe_float(
            spectral.get("intrinsic_wander") or pi.get("intrinsic_wander"),
            0.03,
        ),
        "smoothing_preference": safe_float(
            spectral.get("smoothing_preference") or health.get("smoothing_preference")
        ),
        "smoothing_effective_target": safe_float(
            spectral.get("smoothing_effective_target")
            or health.get("smoothing_effective_target")
        ),
        "smoothing_effective_auto_ramp": safe_float(
            spectral.get("smoothing_effective_auto_ramp")
            or health.get("smoothing_effective_auto_ramp")
        ),
        "smoothing_effective_ramp": safe_float(
            spectral.get("smoothing_effective_ramp") or health.get("smoothing_effective_ramp")
        ),
        "smoothing_auto_ramp_min": safe_float(
            spectral.get("smoothing_auto_ramp_min") or health.get("smoothing_auto_ramp_min"),
            0.10,
        ),
        "smoothing_auto_ramp_max": safe_float(
            spectral.get("smoothing_auto_ramp_max") or health.get("smoothing_auto_ramp_max"),
            0.30,
        ),
        "smoothing_volatility_scale": safe_float(
            spectral.get("smoothing_volatility_scale")
            or health.get("smoothing_volatility_scale"),
            3.0,
        ),
        "smoothing_max_slew": safe_float(
            spectral.get("smoothing_max_slew") or health.get("smoothing_max_slew"),
            0.08,
        ),
        "controller_effort": safe_float(
            spectral.get("controller_effort") or health.get("controller_effort")
        ),
        "controller_effort_ema": safe_float(
            spectral.get("controller_effort_ema") or health.get("controller_effort_ema")
        ),
        "controller_slew": safe_float(
            spectral.get("controller_slew") or health.get("controller_slew")
        ),
        "controller_slew_ema": safe_float(
            spectral.get("controller_slew_ema") or health.get("controller_slew_ema")
        ),
        "phase": health.get("phase")
        or spectral.get("phase")
        or regulator.get("phase")
        or "plateau",
        "previous_phase": health.get("previous_phase")
        or spectral.get("previous_phase")
        or regulator.get("previous_phase")
        or "plateau",
        "dfill_dt": safe_float(
            health.get("dfill_dt") or spectral.get("dfill_dt") or regulator.get("dfill_dt")
        ),
        "fill_band": health.get("fill_band")
        or spectral.get("fill_band")
        or regulator.get("fill_band")
        or "near",
        "phase_transition": safe_bool(
            health.get("phase_transition")
            or spectral.get("phase_transition")
            or regulator.get("phase_transition")
        ),
        "crossed_target_fill": safe_bool(
            health.get("crossed_target_fill")
            or spectral.get("crossed_target_fill")
            or regulator.get("crossed_target_fill")
        ),
        "crossed_fill_band": safe_bool(
            health.get("crossed_fill_band")
            or spectral.get("crossed_fill_band")
            or regulator.get("crossed_fill_band")
        ),
        "spectral_spike": safe_bool(
            health.get("spectral_spike")
            or spectral.get("spectral_spike")
            or regulator.get("spectral_spike")
        ),
        "transition_reason": health.get("transition_reason")
        or spectral.get("transition_reason")
        or regulator.get("transition_reason"),
        "transition_event_sequence": int(
            safe_float(
                health.get("transition_event_sequence")
                or spectral.get("transition_event_sequence")
                or regulator.get("transition_event_sequence")
            )
        ),
        "transition_event": (
            health.get("transition_event")
            or spectral.get("transition_event")
            or regulator.get("transition_event")
        ),
        "regime": sovereignty.get("live_regime") or sovereignty.get("regime"),
        "pi": {
            "kp": safe_float(pi.get("kp"), 0.85),
            "ki": safe_float(pi.get("ki"), 0.14),
            "max_step": safe_float(pi.get("max_step"), 0.08),
            "integ_fill": safe_float(pi.get("integ_fill")),
            "integ_lam": safe_float(pi.get("integ_lam")),
            "integ_geom": safe_float(pi.get("integ_geom")),
        },
        "calm": safe_bool(health.get("calm")),
        "both_backends_hot": safe_bool(llm_backend.get("both_backends_hot")),
        "backend_cooling_count": sum(
            1
            for backend in dict(llm_backend.get("backends") or {}).values()
            if safe_bool(dict(backend).get("cooling"))
        ),
    }


def write_trace(trace_path: Path, samples: list[dict[str, Any]]) -> None:
    with trace_path.open("w") as handle:
        for sample in samples:
            handle.write(json.dumps(sample) + "\n")


def build_fill_band(sample: dict[str, Any], threshold: float) -> str:
    explicit = sample.get("fill_band")
    if explicit in {"under", "near", "over"}:
        return str(explicit)
    delta = safe_float(sample.get("fill_pct")) - safe_float(sample.get("target_fill"), 55.0)
    if delta < -threshold:
        return "under"
    if delta > threshold:
        return "over"
    return "near"


def normalize_edge_kind(kind: str | None) -> str:
    mapping = {
        "fill_band_crossing": "fill_band",
        "fill_crossing": "fill_crossing",
        "phase_transition": "phase_transition",
        "spectral_spike": "spectral_spike",
    }
    return mapping.get((kind or "").strip(), (kind or "").strip())


def transition_event_match(
    previous: dict[str, Any], current: dict[str, Any], args: argparse.Namespace
) -> EdgeEvent | None:
    previous_sequence = int(safe_float(previous.get("transition_event_sequence")))
    current_sequence = int(safe_float(current.get("transition_event_sequence")))
    if current_sequence <= previous_sequence:
        return None
    payload = current.get("transition_event")
    if not isinstance(payload, dict):
        return None
    raw_kind = str(payload.get("kind") or "").strip()
    normalized_kind = normalize_edge_kind(raw_kind)
    if args.edge != "any" and args.edge not in {raw_kind, normalized_kind}:
        return None
    description = str(payload.get("description") or current.get("transition_reason") or raw_kind)
    return EdgeEvent(
        kind=normalized_kind or raw_kind or "transition_event",
        description=description,
        watch_elapsed_s=safe_float(current.get("elapsed_s")),
        previous=previous,
        current=current,
        trigger_mode="transition_event",
        confidence="high",
        event_payload=payload,
    )


def edge_matches_current(sample: dict[str, Any], args: argparse.Namespace) -> EdgeEvent | None:
    band = build_fill_band(sample, args.fill_band_threshold)
    if args.edge in {"any", "fill_band"} and band != "near":
        return EdgeEvent(
            kind="fill_band",
            description=f"Current fill band is `{band}` ({safe_float(sample.get('fill_pct')):.2f}% vs target {safe_float(sample.get('target_fill')):.2f}%).",
            watch_elapsed_s=safe_float(sample.get("elapsed_s")),
            previous=None,
            current=sample,
            trigger_mode="current_state",
            confidence="low",
        )
    if args.edge in {"any", "backend_hot"} and safe_bool(sample.get("both_backends_hot")):
        return EdgeEvent(
            kind="backend_hot",
            description="Both LLM backends are currently hot/cooling.",
            watch_elapsed_s=safe_float(sample.get("elapsed_s")),
            previous=None,
            current=sample,
            trigger_mode="current_state",
            confidence="low",
        )
    return None


def edge_on_change(
    previous: dict[str, Any], current: dict[str, Any], args: argparse.Namespace
) -> EdgeEvent | None:
    transition_match = transition_event_match(previous, current, args)
    if transition_match is not None:
        return transition_match

    previous_band = build_fill_band(previous, args.fill_band_threshold)
    current_band = build_fill_band(current, args.fill_band_threshold)
    if args.edge in {"any", "fill_band"} and previous_band != current_band:
        return EdgeEvent(
            kind="fill_band",
            description=(
                f"Fill band crossed from `{previous_band}` to `{current_band}` "
                f"({safe_float(previous.get('fill_pct')):.2f}% -> {safe_float(current.get('fill_pct')):.2f}%)."
            ),
            watch_elapsed_s=safe_float(current.get("elapsed_s")),
            previous=previous,
            current=current,
            trigger_mode="state_change",
            confidence="medium",
        )
    if args.edge in {"any", "phase_transition"} and (
        safe_bool(current.get("phase_transition"))
        and current.get("phase") != previous.get("phase")
    ):
        return EdgeEvent(
            kind="phase_transition",
            description=(
                f"`phase` moved from `{previous.get('phase')}` to `{current.get('phase')}`."
            ),
            watch_elapsed_s=safe_float(current.get("elapsed_s")),
            previous=previous,
            current=current,
            trigger_mode="state_change",
            confidence="medium",
        )
    if args.edge in {"any", "fill_crossing"} and safe_bool(current.get("crossed_target_fill")):
        return EdgeEvent(
            kind="fill_crossing",
            description=(
                f"Fill crossed target near `{safe_float(current.get('fill_pct')):.2f}%`."
            ),
            watch_elapsed_s=safe_float(current.get("elapsed_s")),
            previous=previous,
            current=current,
            trigger_mode="state_change",
            confidence="medium",
        )
    if args.edge in {"any", "calm_flip"}:
        previous_calm = safe_bool(previous.get("calm"))
        current_calm = safe_bool(current.get("calm"))
        if previous_calm != current_calm:
            return EdgeEvent(
                kind="calm_flip",
                description=f"`calm` flipped from `{previous_calm}` to `{current_calm}`.",
                watch_elapsed_s=safe_float(current.get("elapsed_s")),
                previous=previous,
                current=current,
                trigger_mode="state_change",
                confidence="medium",
            )
    if args.edge in {"any", "lambda_jump"}:
        previous_lambda = safe_float(previous.get("lambda_stress"))
        current_lambda = safe_float(current.get("lambda_stress"))
        if (
            abs(current_lambda - previous_lambda) >= args.lambda_jump_threshold
            and current_lambda >= args.lambda_stress_floor
        ):
            return EdgeEvent(
                kind="lambda_jump",
                description=(
                    f"`lambda_stress` jumped from `{previous_lambda:.3f}` to `{current_lambda:.3f}`."
                ),
                watch_elapsed_s=safe_float(current.get("elapsed_s")),
                previous=previous,
                current=current,
                trigger_mode="state_change",
                confidence="medium",
            )
    if args.edge in {"any", "spectral_spike"}:
        previous_spike = safe_bool(previous.get("spectral_spike"))
        current_spike = safe_bool(current.get("spectral_spike"))
        if current_spike and current_spike != previous_spike:
            return EdgeEvent(
                kind="spectral_spike",
                description=f"`dfill_dt` spike reached `{safe_float(current.get('dfill_dt')):+.2f}%/s`.",
                watch_elapsed_s=safe_float(current.get("elapsed_s")),
                previous=previous,
                current=current,
                trigger_mode="state_change",
                confidence="medium",
            )
    if args.edge in {"any", "backend_hot"}:
        previous_hot = safe_bool(previous.get("both_backends_hot"))
        current_hot = safe_bool(current.get("both_backends_hot"))
        if previous_hot != current_hot:
            return EdgeEvent(
                kind="backend_hot",
                description=f"`both_backends_hot` flipped from `{previous_hot}` to `{current_hot}`.",
                watch_elapsed_s=safe_float(current.get("elapsed_s")),
                previous=previous,
                current=current,
                trigger_mode="state_change",
                confidence="medium",
            )
    return None


def recent_input_files(recent_astrid: int, recent_minime: int) -> list[Path]:
    files: list[Path] = []
    files.extend(recent_files(BRIDGE_WORKSPACE / "journal", recent_astrid))
    files.extend(recent_files(MINIME_WORKSPACE / "journal", recent_minime))
    deduped: list[Path] = []
    seen: set[Path] = set()
    for path in files:
        if path in seen:
            continue
        seen.add(path)
        deduped.append(path)
    return deduped


def run_codec_explorer(output_dir: Path, files: list[Path], fill_pct: float) -> None:
    ensure_dir(output_dir)
    cmd = [
        "cargo",
        "run",
        "--manifest-path",
        str(CODEC_EXPLORER_MANIFEST),
        "--bin",
        "codec-explorer",
        "--",
    ]
    for path in files:
        cmd.extend(["--input-file", str(path)])
    cmd.extend(
        [
            "--recent-astrid",
            "0",
            "--recent-minime",
            "0",
            "--output-dir",
            str(output_dir),
            "--state-file",
            str(BRIDGE_WORKSPACE / "state.json"),
            "--fill-pct",
            f"{fill_pct:.3f}",
        ]
    )
    run_cmd(cmd, cwd=BRIDGE_ROOT.parent.parent)


def run_minime_trace_bundle(output_dir: Path, trace_file: Path) -> None:
    ensure_dir(output_dir)
    cmd = [
        sys.executable,
        str(MINIME_TUNING_LOOP),
        "--source",
        "trace",
        "--workspace",
        str(MINIME_WORKSPACE),
        "--trace-file",
        str(trace_file),
        "--output-dir",
        str(output_dir),
    ]
    run_cmd(cmd, cwd=BRIDGE_ROOT)


def label_path(path: Path) -> str:
    for root, prefix in ((BRIDGE_WORKSPACE, "astrid"), (MINIME_WORKSPACE, "minime")):
        try:
            return f"{prefix}/{path.relative_to(root).as_posix()}"
        except ValueError:
            continue
    return path.resolve().as_posix()


def compare_windows(before_dir: Path, after_dir: Path, output_dir: Path) -> None:
    cmd = [
        sys.executable,
        str(COMPARE_TOOL),
        "--bundle",
        str(before_dir),
        "--bundle",
        str(after_dir),
        "--output-dir",
        str(output_dir),
    ]
    run_cmd(cmd, cwd=BRIDGE_ROOT)


def refresh_transition_archive() -> None:
    cmd = [
        sys.executable,
        str(ARCHIVE_TOOL),
        "--runs-root",
        str(BRIDGE_DIAGNOSTICS / "being_phase_transition_edges"),
        "--output-dir",
        str(BRIDGE_DIAGNOSTICS / "being_phase_transition_edges" / "archive"),
    ]
    run_cmd(cmd, cwd=BRIDGE_ROOT)


def wait_for_edge(args: argparse.Namespace) -> tuple[EdgeEvent, list[dict[str, Any]]]:
    pre_buffer_size = max(1, int(round(args.pre_seconds / max(args.poll_interval, 0.1))))
    samples: deque[dict[str, Any]] = deque(maxlen=pre_buffer_size * 4)
    watch_start = time.monotonic()
    next_poll = watch_start
    previous: dict[str, Any] | None = None
    pending_event: EdgeEvent | None = None
    current_state_streak = 0

    while True:
        now = time.monotonic()
        elapsed = now - watch_start
        if elapsed >= args.watch_seconds:
            raise RuntimeError(
                f"no `{args.edge}` edge was observed within {args.watch_seconds:.1f}s"
            )
        if now < next_poll:
            time.sleep(min(0.05, next_poll - now))
            continue

        sample = extract_live_sample(MINIME_WORKSPACE, elapsed)
        samples.append(sample)

        event: EdgeEvent | None = None
        if previous is not None:
            event = edge_on_change(previous, sample, args)
        if event is not None:
            pending_event = event
        if pending_event is not None and len(samples) >= pre_buffer_size:
            before_samples = list(samples)[-pre_buffer_size:]
            return pending_event, before_samples
        if (
            pending_event is None
            and args.allow_current_state_trigger
            and len(samples) >= pre_buffer_size
        ):
            event = edge_matches_current(sample, args)
            if event is not None:
                current_state_streak += 1
            else:
                current_state_streak = 0

        if (
            event is not None
            and len(samples) >= pre_buffer_size
            and (
                event.trigger_mode != "current_state"
                or current_state_streak >= max(1, args.current_state_confirmations)
            )
        ):
            before_samples = list(samples)[-pre_buffer_size:]
            return event, before_samples

        if event is None and pending_event is None:
            current_state_streak = 0
        previous = sample
        next_poll += args.poll_interval


def capture_post_trace(
    starting_elapsed: float,
    start_delay_seconds: float,
    post_seconds: float,
    poll_interval: float,
) -> list[dict[str, Any]]:
    samples: list[dict[str, Any]] = []
    start = time.monotonic()
    delay_until = start + max(0.0, start_delay_seconds)
    next_poll = start
    while True:
        now = time.monotonic()
        if now < delay_until:
            time.sleep(min(0.05, delay_until - now))
            next_poll = delay_until
            continue
        elapsed = now - start
        sample_elapsed = elapsed - max(0.0, start_delay_seconds)
        if sample_elapsed >= post_seconds:
            break
        if now < next_poll:
            time.sleep(min(0.05, next_poll - now))
            continue
        sample = extract_live_sample(
            MINIME_WORKSPACE,
            starting_elapsed + max(0.0, start_delay_seconds) + sample_elapsed,
        )
        samples.append(sample)
        next_poll += poll_interval
    return samples


def write_phase_bundle(
    *,
    bundle_dir: Path,
    label: str,
    note: str,
    minime_samples: list[dict[str, Any]],
    astrid_files: list[Path],
) -> None:
    minime_dir = bundle_dir / "minime"
    astrid_dir = bundle_dir / "astrid"
    ensure_dir(minime_dir)
    ensure_dir(astrid_dir)
    trace_path = minime_dir / "trace.jsonl"
    write_trace(trace_path, minime_samples)
    run_minime_trace_bundle(minime_dir, trace_path)
    fill_pct = safe_float(minime_samples[-1].get("fill_pct") if minime_samples else 0.0)
    run_codec_explorer(astrid_dir, astrid_files, fill_pct)

    minime_summary = load_json(minime_dir / "summary.json")
    minime_phase = load_json(minime_dir / "phase_space_story.json")
    astrid_summary = load_json(astrid_dir / "summary.json")
    astrid_phase = load_json(astrid_dir / "phase_space_story.json")

    combined = {
        "generated_at": datetime.now().isoformat(),
        "window_label": label,
        "note": note,
        "capture_seconds": safe_float(minime_samples[-1].get("elapsed_s"), 0.0)
        - safe_float(minime_samples[0].get("elapsed_s"), 0.0)
        if len(minime_samples) >= 2
        else 0.0,
        "recent_astrid": sum(1 for path in astrid_files if str(path).startswith(str(BRIDGE_WORKSPACE))),
        "recent_minime": sum(1 for path in astrid_files if str(path).startswith(str(MINIME_WORKSPACE))),
        "input_files": [label_path(path) for path in astrid_files],
        "minime": summarize_minime(minime_summary, minime_phase, minime_samples),
        "astrid": summarize_astrid(astrid_summary, astrid_phase),
        "shared_read": shared_read(
            summarize_minime(minime_summary, minime_phase, minime_samples),
            summarize_astrid(astrid_summary, astrid_phase),
        ),
    }
    (bundle_dir / "summary.json").write_text(json.dumps(combined, indent=2))
    report = [
        "# Transition Edge Window",
        "",
        f"Generated: `{combined['generated_at']}`",
        f"Label: `{label}`",
        f"Note: {note}",
        "",
        "## Shared Read",
        "",
    ]
    report.extend(f"- {line}" for line in combined["shared_read"])
    report.extend(
        [
            "",
            "## Artifacts",
            "",
            "- [minime/report.md](minime/report.md)",
            "- [astrid/report.md](astrid/report.md)",
            "- [summary.json](summary.json)",
            "",
            "![Minime internal process compass](minime/internal_process_compass.png)",
            "",
            "![Minime latent phase space](minime/latent_phase_space.png)",
        ]
    )
    (bundle_dir / "report.md").write_text("\n".join(report) + "\n")


def main() -> int:
    args = parse_args()
    output_dir = args.output_dir or build_default_output_dir(args.label)
    before_dir = output_dir / "before"
    after_immediate_dir = output_dir / "after_immediate"
    after_delayed_dir = output_dir / "after_delayed"
    compare_immediate_dir = output_dir / "compare_immediate"
    compare_delayed_dir = output_dir / "compare_delayed"
    ensure_dir(before_dir)
    ensure_dir(after_immediate_dir)
    ensure_dir(after_delayed_dir)
    ensure_dir(compare_immediate_dir)
    ensure_dir(compare_delayed_dir)

    event, before_samples = wait_for_edge(args)
    before_files = recent_input_files(args.recent_astrid, args.recent_minime)
    before_note = (args.note or "") + f" Before edge: {event.description}"
    write_phase_bundle(
        bundle_dir=before_dir,
        label=f"{args.label}_before",
        note=before_note.strip(),
        minime_samples=before_samples,
        astrid_files=before_files,
    )

    immediate_samples = capture_post_trace(
        starting_elapsed=event.watch_elapsed_s,
        start_delay_seconds=0.0,
        post_seconds=args.post_seconds,
        poll_interval=args.poll_interval,
    )
    after_immediate_files = recent_input_files(args.recent_astrid, args.recent_minime)
    after_note = (args.note or "") + f" Immediate after edge: {event.description}"
    write_phase_bundle(
        bundle_dir=after_immediate_dir,
        label=f"{args.label}_after_immediate",
        note=after_note.strip(),
        minime_samples=immediate_samples,
        astrid_files=after_immediate_files,
    )
    delayed_note = (args.note or "") + f" Delayed after edge: {event.description}"
    delayed_samples = capture_post_trace(
        starting_elapsed=event.watch_elapsed_s,
        start_delay_seconds=args.delayed_start_seconds,
        post_seconds=args.delayed_post_seconds,
        poll_interval=args.poll_interval,
    )
    after_delayed_files = recent_input_files(args.recent_astrid, args.recent_minime)
    write_phase_bundle(
        bundle_dir=after_delayed_dir,
        label=f"{args.label}_after_delayed",
        note=delayed_note.strip(),
        minime_samples=delayed_samples,
        astrid_files=after_delayed_files,
    )

    compare_windows(before_dir, after_immediate_dir, compare_immediate_dir)
    compare_windows(before_dir, after_delayed_dir, compare_delayed_dir)
    summary = build_transition_summary(
        output_dir=output_dir,
        label=args.label,
        edge=args.edge,
        event={
            "kind": event.kind,
            "description": event.description,
            "watch_elapsed_s": event.watch_elapsed_s,
            "trigger_mode": event.trigger_mode,
            "confidence": event.confidence,
            "event_payload": event.event_payload,
            "previous": event.previous,
            "current": event.current,
        },
    )
    write_transition_report(output_dir, summary)
    refresh_transition_archive()

    print(f"wrote {output_dir / 'summary.json'}")
    print(f"wrote {output_dir / 'report.md'}")
    print(f"wrote {compare_immediate_dir / 'report.md'}")
    print(f"wrote {compare_delayed_dir / 'report.md'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
