#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path
from typing import Any


BRIDGE_ROOT = Path("/Users/v/other/astrid/capsules/consciousness-bridge")
BRIDGE_WORKSPACE = BRIDGE_ROOT / "workspace"
BRIDGE_DIAGNOSTICS = BRIDGE_WORKSPACE / "diagnostics"
WATCHER_TOOL = BRIDGE_ROOT / "tools" / "being_phase_space_watcher.py"
COMPARE_TOOL = BRIDGE_ROOT / "tools" / "being_phase_space_compare.py"
SPEAK_TO_MINIME_TOOL = BRIDGE_ROOT / "tools" / "speak_to_minime.py"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Capture before/after being phase-space windows around a prompt input."
    )
    parser.add_argument("prompt", type=str, help="Prompt text to inject.")
    parser.add_argument(
        "--target",
        choices=("minime", "astrid", "both"),
        default="minime",
        help="Where to inject the prompt between the before/after captures.",
    )
    parser.add_argument(
        "--label",
        type=str,
        default="prompt_transition",
        help="Label prefix for the transition bundle.",
    )
    parser.add_argument(
        "--pre-seconds",
        type=float,
        default=10.0,
        help="Length of the capture window before the prompt.",
    )
    parser.add_argument(
        "--post-seconds",
        type=float,
        default=10.0,
        help="Length of the capture window after the prompt.",
    )
    parser.add_argument(
        "--capture-interval",
        type=float,
        default=1.0,
        help="Sampling interval for both before/after windows.",
    )
    parser.add_argument(
        "--recent-astrid",
        type=int,
        default=4,
        help="How many recent Astrid journals each watcher window should include.",
    )
    parser.add_argument(
        "--recent-minime",
        type=int,
        default=2,
        help="How many recent Minime journals each watcher window should include.",
    )
    parser.add_argument(
        "--post-delay",
        type=float,
        default=1.0,
        help="Seconds to wait after prompt injection before beginning the post window.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help="Output directory. Defaults to workspace/diagnostics/being_phase_space_transitions/<timestamp>_<label>/.",
    )
    return parser.parse_args()


def ensure_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)


def load_json(path: Path) -> dict[str, Any]:
    try:
        return json.loads(path.read_text())
    except Exception:
        return {}


def run_cmd(cmd: list[str], cwd: Path) -> None:
    subprocess.run(cmd, cwd=cwd, check=True)


def build_default_output_dir(label: str) -> Path:
    stamp = datetime.now().strftime("%Y%m%dT%H%M%S")
    safe = "".join(ch.lower() if ch.isalnum() else "_" for ch in label).strip("_") or "prompt_transition"
    return BRIDGE_DIAGNOSTICS / "being_phase_space_transitions" / f"{stamp}_{safe}"


def run_watcher(
    *,
    output_dir: Path,
    label: str,
    note: str,
    capture_seconds: float,
    capture_interval: float,
    recent_astrid: int,
    recent_minime: int,
) -> None:
    cmd = [
        sys.executable,
        str(WATCHER_TOOL),
        "--capture-seconds",
        f"{capture_seconds:.3f}",
        "--capture-interval",
        f"{capture_interval:.3f}",
        "--recent-astrid",
        str(recent_astrid),
        "--recent-minime",
        str(recent_minime),
        "--label",
        label,
        "--note",
        note,
        "--output-dir",
        str(output_dir),
    ]
    run_cmd(cmd, cwd=BRIDGE_ROOT)


def inject_minime_prompt(prompt: str) -> None:
    cmd = [sys.executable, str(SPEAK_TO_MINIME_TOOL), prompt]
    run_cmd(cmd, cwd=BRIDGE_ROOT)


def inject_astrid_prompt(prompt: str, label: str) -> Path:
    inbox_dir = BRIDGE_WORKSPACE / "inbox"
    ensure_dir(inbox_dir)
    stamp = datetime.now().strftime("%Y%m%dT%H%M%S")
    path = inbox_dir / f"phase_space_prompt_{stamp}.txt"
    body = "\n".join(
        [
            "=== PHASE-SPACE PROMPT ===",
            f"Timestamp: {datetime.now().isoformat()}",
            "Source: steward",
            f"Label: {label}",
            "",
            prompt,
            "",
            "This note was placed to observe before/after phase-space movement.",
        ]
    )
    path.write_text(body + "\n")
    return path


def inject_prompt(prompt: str, target: str, label: str) -> dict[str, Any]:
    details: dict[str, Any] = {"target": target, "prompt": prompt}
    if target in {"minime", "both"}:
        inject_minime_prompt(prompt)
        details["minime_sent"] = True
    if target in {"astrid", "both"}:
        details["astrid_inbox_path"] = str(inject_astrid_prompt(prompt, label))
    return details


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


def summarize_transition(before_dir: Path, after_dir: Path, compare_dir: Path, args: argparse.Namespace, injection: dict[str, Any]) -> dict[str, Any]:
    before = load_json(before_dir / "summary.json")
    after = load_json(after_dir / "summary.json")
    compare = load_json(compare_dir / "summary.json")
    before_fill = float((before.get("minime") or {}).get("current_fill_pct") or 0.0)
    after_fill = float((after.get("minime") or {}).get("current_fill_pct") or 0.0)
    before_minime_pc1 = float(((before.get("minime") or {}).get("phase_space_explained_variance") or [0.0])[0])
    after_minime_pc1 = float(((after.get("minime") or {}).get("phase_space_explained_variance") or [0.0])[0])
    before_astrid_pc1 = float(((before.get("astrid") or {}).get("phase_space_explained_variance") or [0.0])[0])
    after_astrid_pc1 = float(((after.get("astrid") or {}).get("phase_space_explained_variance") or [0.0])[0])
    summary = {
        "generated_at": datetime.now().isoformat(),
        "label": args.label,
        "prompt": args.prompt,
        "target": args.target,
        "pre_seconds": args.pre_seconds,
        "post_seconds": args.post_seconds,
        "capture_interval": args.capture_interval,
        "post_delay": args.post_delay,
        "injection": injection,
        "before_dir": str(before_dir),
        "after_dir": str(after_dir),
        "compare_dir": str(compare_dir),
        "before_fill_pct": before_fill,
        "after_fill_pct": after_fill,
        "delta_fill_pct": after_fill - before_fill,
        "before_minime_pc1": before_minime_pc1,
        "after_minime_pc1": after_minime_pc1,
        "before_astrid_pc1": before_astrid_pc1,
        "after_astrid_pc1": after_astrid_pc1,
        "compare_notes": list(compare.get("notes") or []),
    }
    return summary


def write_transition_bundle(output_dir: Path, summary: dict[str, Any]) -> None:
    (output_dir / "summary.json").write_text(json.dumps(summary, indent=2))
    report = [
        "# Prompt Transition Phase-Space Watcher",
        "",
        f"Generated: `{summary['generated_at']}`",
        f"Label: `{summary['label']}`",
        f"Target: `{summary['target']}`",
        f"Prompt: `{summary['prompt']}`",
        "",
        "## Read",
        "",
        f"- Minime fill: `{summary['before_fill_pct']:.2f}%` -> `{summary['after_fill_pct']:.2f}%` (`{summary['delta_fill_pct']:+.2f}`)",
        f"- Minime PC1: `{summary['before_minime_pc1']:.3f}` -> `{summary['after_minime_pc1']:.3f}`",
        f"- Astrid PC1: `{summary['before_astrid_pc1']:.3f}` -> `{summary['after_astrid_pc1']:.3f}`",
    ]
    for line in summary.get("compare_notes") or []:
        report.append(f"- {line}")
    report.extend(
        [
            "",
            "## Bundles",
            "",
            f"- Before: [report.md](before/report.md)",
            f"- After: [report.md](after/report.md)",
            f"- Comparison: [report.md](compare/report.md)",
            "",
            "![Before Minime phase space](before/minime/phase_space_projection.png)",
            "",
            "![After Minime phase space](after/minime/phase_space_projection.png)",
        ]
    )
    (output_dir / "report.md").write_text("\n".join(report) + "\n")


def main() -> int:
    args = parse_args()
    output_dir = args.output_dir or build_default_output_dir(args.label)
    before_dir = output_dir / "before"
    after_dir = output_dir / "after"
    compare_dir = output_dir / "compare"
    ensure_dir(before_dir)
    ensure_dir(after_dir)
    ensure_dir(compare_dir)

    run_watcher(
        output_dir=before_dir,
        label=f"{args.label}_before",
        note=f"Before prompt transition: {args.prompt}",
        capture_seconds=args.pre_seconds,
        capture_interval=args.capture_interval,
        recent_astrid=args.recent_astrid,
        recent_minime=args.recent_minime,
    )

    injection = inject_prompt(args.prompt, args.target, args.label)
    if args.post_delay > 0:
        time.sleep(args.post_delay)

    run_watcher(
        output_dir=after_dir,
        label=f"{args.label}_after",
        note=f"After prompt transition: {args.prompt}",
        capture_seconds=args.post_seconds,
        capture_interval=args.capture_interval,
        recent_astrid=args.recent_astrid,
        recent_minime=args.recent_minime,
    )

    compare_windows(before_dir, after_dir, compare_dir)
    summary = summarize_transition(before_dir, after_dir, compare_dir, args, injection)
    write_transition_bundle(output_dir, summary)

    print(f"wrote {output_dir / 'summary.json'}")
    print(f"wrote {output_dir / 'report.md'}")
    print(f"wrote {compare_dir / 'report.md'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
