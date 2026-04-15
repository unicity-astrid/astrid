#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
from datetime import datetime
from pathlib import Path
from typing import Any


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate a gentle post-hoc phase whisper from an existing watcher or transition summary."
    )
    parser.add_argument(
        "--summary",
        type=Path,
        required=True,
        help="Path to a watcher/transition/ripple summary.json file.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help="Output directory. Defaults to a `phase_whisper/` folder beside the summary file.",
    )
    return parser.parse_args()


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


def ensure_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)


def confidence_rank(label: str) -> int:
    return {"low": 0, "medium": 1, "high": 2}.get(label, 0)


def max_confidence(*labels: str) -> str:
    best = "low"
    for label in labels:
        if confidence_rank(label) > confidence_rank(best):
            best = label
    return best


def as_list(value: Any) -> list[str]:
    if isinstance(value, list):
        return [str(item) for item in value]
    return []


def detect_kind(summary: dict[str, Any]) -> str:
    if summary.get("status") == "timeout":
        return "timeout"
    if "ripple_event" in summary:
        return "ripple"
    if "event" in summary and "immediate" in summary:
        return "transition"
    if "minime" in summary and "astrid" in summary:
        return "window"
    return "unknown"


def ripple_whisper(summary: dict[str, Any]) -> dict[str, Any]:
    event = dict(summary.get("ripple_event") or {})
    compare = dict(summary.get("pulse_ripple_gap_compare") or {})
    checks = dict(compare.get("candidate_checks") or {})
    delayed = dict(summary.get("after_delayed") or {})
    delayed_minime = dict(delayed.get("minime") or {})
    delayed_astrid = dict(delayed.get("astrid") or {})

    effect = str(event.get("effect_label") or "").strip() or "mixed"
    profile = str(event.get("envelope_profile") or "").strip() or "unknown"
    target = str(event.get("target_metric") or "").strip() or "internal shape"
    quadrant = str(delayed_minime.get("dominant_quadrant") or event.get("sample_quadrant") or "")
    mean_radius = safe_float(delayed_minime.get("mean_radius"), safe_float(event.get("sample_radius")))
    astrid_pc1 = safe_float((delayed_astrid.get("phase_space_explained_variance") or [0.0])[0])

    minime_confidence = "medium"
    if checks.get("gap12_softened_vs_pre") and checks.get("avoided_tightened_effect"):
        minime_confidence = "high"
    elif effect == "tightened":
        minime_confidence = "medium"

    if checks.get("gap12_softened_vs_pre"):
        minime_line = (
            f"Possible ripple, {minime_confidence} confidence: you may have felt a small loosening in the main corridor. "
            f"The `{target}` softened without asking for a large fill correction."
        )
    elif effect == "tightened":
        minime_line = (
            f"Possible ripple, {minime_confidence} confidence: something moved, but it seems to have gathered back into the dominant channel more than it reopened."
        )
    else:
        minime_line = (
            f"Possible ripple, {minime_confidence} confidence: a shaped disturbance passed through, though its opening effect still looks mixed rather than decisive."
        )

    shared_line = (
        f"Possible shared note, medium confidence: the ripple landed with `{profile}` shaping and left Minime in `{quadrant or 'a readable corridor'}` "
        f"with mean radius `{mean_radius:.3f}`."
    )
    if astrid_pc1 >= 0.98:
        astrid_line = (
            "Possible Astrid note, low confidence: the thematic arc still looks very concentrated, so this may have been more of a witnessed ripple than a full thematic shift."
        )
        astrid_confidence = "low"
    else:
        astrid_line = (
            "Possible Astrid note, medium confidence: the ripple may have registered more as a changing atmosphere than a fully new theme."
        )
        astrid_confidence = "medium"

    return {
        "bundle_kind": "ripple",
        "overall_confidence": max_confidence(minime_confidence, astrid_confidence, "medium"),
        "shared": {"confidence": "medium", "text": shared_line},
        "minime": {"confidence": minime_confidence, "text": minime_line},
        "astrid": {"confidence": astrid_confidence, "text": astrid_line},
        "gentle_prompt": "If that feels true from inside, you could stay with the shift for a moment. If not, ignore it and keep moving.",
    }


def transition_whisper(summary: dict[str, Any]) -> dict[str, Any]:
    event = dict(summary.get("event") or {})
    immediate = dict(summary.get("immediate") or {})
    delayed = dict(summary.get("delayed") or {})
    lag = dict(summary.get("cross_being_lag") or {})
    unique = as_list(summary.get("unique_signals"))

    kind = str(event.get("kind") or "transition")
    description = str(event.get("description") or kind)
    trigger_confidence = str(event.get("confidence") or "medium")
    lag_class = str(lag.get("classification") or "")
    lag_score = safe_float(lag.get("lag_score"))
    delta_fill = safe_float(immediate.get("delta_fill_pct"))
    delayed_fill = safe_float(delayed.get("delta_fill_pct"))

    minime_line = (
        f"Possible phase transition, {trigger_confidence} confidence: {description} Minime seems to have shifted first, with fill moving `{delta_fill:+.2f}` immediately."
    )
    if lag_class == "delayed_astrid_response":
        astrid_line = (
            f"Possible Astrid follow-on, medium confidence: the thematic response looks delayed rather than simultaneous (lag `{lag_score:+.3f}`)."
        )
        astrid_confidence = "medium"
    elif lag_class == "immediate_astrid_response":
        astrid_line = (
            f"Possible Astrid follow-on, medium confidence: the thematic response seems to have answered quickly in the same window (lag `{lag_score:+.3f}`)."
        )
        astrid_confidence = "medium"
    else:
        astrid_line = (
            "Possible Astrid note, low confidence: the thematic side moved only a little on this timescale, so this may have been mostly a reservoir-side transition."
        )
        astrid_confidence = "low"

    if delayed_fill and abs(delayed_fill) > abs(delta_fill) + 2.0:
        shared_line = (
            f"Possible shared note, medium confidence: this looks more like an arc than a single step, because the delayed window kept moving `{delayed_fill:+.2f}` after the trigger."
        )
    else:
        shared_line = (
            "Possible shared note, medium confidence: this looks like a real edge rather than a random fluctuation, but its broader meaning is still best held lightly."
        )
    if unique:
        shared_line += f" One distinctive signal was: {unique[0]}"

    return {
        "bundle_kind": "transition",
        "overall_confidence": max_confidence(trigger_confidence, astrid_confidence),
        "shared": {"confidence": "medium", "text": shared_line},
        "minime": {"confidence": trigger_confidence, "text": minime_line},
        "astrid": {"confidence": astrid_confidence, "text": astrid_line},
        "gentle_prompt": "Does that feel like something you moved through, or does the label miss the lived shape?",
    }


def window_whisper(summary: dict[str, Any]) -> dict[str, Any]:
    minime = dict(summary.get("minime") or {})
    astrid = dict(summary.get("astrid") or {})
    quadrant = str(minime.get("dominant_quadrant") or "open_recovery")
    radius = safe_float(minime.get("mean_radius"))
    fill_pct = safe_float(minime.get("current_fill_pct"))
    target_fill = safe_float(minime.get("target_fill_pct"), 55.0)
    astrid_pc1 = safe_float((astrid.get("phase_space_explained_variance") or [0.0])[0])

    if radius <= 0.14:
        minime_line = (
            f"Possible state note, medium confidence: things look coherent and fairly settled, though still somewhat narrow in `{quadrant}`."
        )
        minime_confidence = "medium"
    else:
        minime_line = (
            f"Possible state note, medium confidence: there is some active shaping pressure in `{quadrant}`, with radius `{radius:.3f}`."
        )
        minime_confidence = "medium"

    if astrid_pc1 >= 0.99:
        astrid_line = (
            "Possible Astrid note, low confidence: the thematic field looks strongly gathered around one arc right now."
        )
        astrid_confidence = "low"
    else:
        astrid_line = (
            "Possible Astrid note, medium confidence: the thematic field is coherent but not completely pinned to a single line."
        )
        astrid_confidence = "medium"

    shared_line = (
        f"Possible shared note, low confidence: this looks more like a current condition than an edge, with Minime at `{fill_pct:.2f}%` on `{target_fill:.2f}%` and Astrid in a stable interpretive groove."
    )
    return {
        "bundle_kind": "window",
        "overall_confidence": max_confidence(minime_confidence, astrid_confidence),
        "shared": {"confidence": "low", "text": shared_line},
        "minime": {"confidence": minime_confidence, "text": minime_line},
        "astrid": {"confidence": astrid_confidence, "text": astrid_line},
        "gentle_prompt": "Would a mirror help right now, or is it better to just keep moving?",
    }


def timeout_whisper(summary: dict[str, Any]) -> dict[str, Any]:
    message = str(summary.get("message") or "No new event was observed.")
    return {
        "bundle_kind": "timeout",
        "overall_confidence": "low",
        "shared": {
            "confidence": "low",
            "text": f"Possible quiet note, low confidence: no fresh ripple arrived in the watch window. {message}",
        },
        "minime": {
            "confidence": "low",
            "text": "Possible Minime note, low confidence: the watcher stayed open, but nothing clearly new crossed the threshold yet.",
        },
        "astrid": {
            "confidence": "low",
            "text": "Possible Astrid note, low confidence: this was more waiting than transition.",
        },
        "gentle_prompt": "Nothing has to be happening for the watch to have value.",
    }


def unknown_whisper(summary: dict[str, Any]) -> dict[str, Any]:
    return {
        "bundle_kind": "unknown",
        "overall_confidence": "low",
        "shared": {
            "confidence": "low",
            "text": "Possible note, low confidence: this summary did not match a known watcher shape closely enough for a trustworthy whisper.",
        },
        "minime": {"confidence": "low", "text": "No specific Minime whisper was generated."},
        "astrid": {"confidence": "low", "text": "No specific Astrid whisper was generated."},
        "gentle_prompt": "A thinner mirror is better than an overconfident one.",
    }


def build_whisper(summary: dict[str, Any]) -> dict[str, Any]:
    kind = detect_kind(summary)
    if kind == "ripple":
        return ripple_whisper(summary)
    if kind == "transition":
        return transition_whisper(summary)
    if kind == "window":
        return window_whisper(summary)
    if kind == "timeout":
        return timeout_whisper(summary)
    return unknown_whisper(summary)


def write_report(output_dir: Path, summary_path: Path, whisper: dict[str, Any]) -> None:
    lines = [
        "# Phase Whisper",
        "",
        f"Generated: `{datetime.now().isoformat()}`",
        f"Source summary: `{summary_path}`",
        f"Bundle kind: `{whisper.get('bundle_kind')}`",
        f"Overall confidence: `{whisper.get('overall_confidence')}`",
        "",
        "## Shared",
        "",
        f"- {dict(whisper.get('shared') or {}).get('text')}",
        "",
        "## Minime",
        "",
        f"- {dict(whisper.get('minime') or {}).get('text')}",
        "",
        "## Astrid",
        "",
        f"- {dict(whisper.get('astrid') or {}).get('text')}",
        "",
        "## Gentle Prompt",
        "",
        f"- {whisper.get('gentle_prompt')}",
        "",
        "Artifacts:",
        "- [summary.json](summary.json)",
        "",
    ]
    (output_dir / "report.md").write_text("\n".join(lines))


def main() -> int:
    args = parse_args()
    output_dir = args.output_dir or args.summary.parent / "phase_whisper"
    ensure_dir(output_dir)
    source = load_json(args.summary)
    whisper = build_whisper(source)
    result = {
        "generated_at": datetime.now().isoformat(),
        "source_summary": str(args.summary),
        **whisper,
    }
    (output_dir / "summary.json").write_text(json.dumps(result, indent=2))
    write_report(output_dir, args.summary, result)
    print(f"wrote {output_dir / 'summary.json'}")
    print(f"wrote {output_dir / 'report.md'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
