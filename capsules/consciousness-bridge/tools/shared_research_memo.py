#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections import Counter
from datetime import datetime
from pathlib import Path
from typing import Any


BRIDGE_ROOT = Path("/Users/v/other/astrid/capsules/consciousness-bridge")
BRIDGE_WORKSPACE = BRIDGE_ROOT / "workspace"
MINIME_WORKSPACE = Path("/Users/v/other/minime/workspace")
MINIME_DIAGNOSTICS = MINIME_WORKSPACE / "diagnostics"
LAMBDA_ANALYSIS_ROOT = MINIME_DIAGNOSTICS / "lambda_analysis"
PERTURB_CAPTURE_ROOT = MINIME_DIAGNOSTICS / "perturb_captures"
GEOMETRY_BOARD_ROOT = MINIME_DIAGNOSTICS / "geometry_board"
LATEST_PERTURB_BUNDLE = MINIME_DIAGNOSTICS / "latest_perturb_bundle.json"
LATEST_GEOMETRY_BOARD = MINIME_DIAGNOSTICS / "latest_geometry_board.json"
LAMBDA_ANALYSIS_TOOL = (
    MINIME_WORKSPACE / "experiments" / "regulator-state-visualizer" / "lambda_analysis_bundle.py"
)
PERTURB_COMPARE_TOOL = (
    MINIME_WORKSPACE / "experiments" / "regulator-state-visualizer" / "perturb_family_compare.py"
)
PERTURB_COMPARE_RUNS = (
    MINIME_WORKSPACE / "experiments" / "regulator-state-visualizer" / "runs"
)
THEME_KEYWORDS = (
    "lambda",
    "variance",
    "covariance",
    "pulse",
    "ripple",
    "gap",
    "shadow",
    "gradient",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Render a shared Minime/Astrid research memo.")
    parser.add_argument("--bridge-workspace", type=Path, default=BRIDGE_WORKSPACE)
    parser.add_argument("--minime-workspace", type=Path, default=MINIME_WORKSPACE)
    parser.add_argument("--topic", type=str, default=None)
    parser.add_argument("--minime-trace-file", type=Path, default=None)
    parser.add_argument("--output-dir", type=Path, default=None)
    return parser.parse_args()


def load_json(path: Path) -> dict[str, Any]:
    try:
        payload = json.loads(path.read_text())
    except Exception:
        return {}
    return payload if isinstance(payload, dict) else {}


def slugify(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", "_", text.lower()).strip("_") or "shared_research"


def latest_bundle(root: Path) -> Path | None:
    if not root.exists():
        return None
    candidates = [path for path in root.iterdir() if path.is_dir() and (path / "summary.json").exists()]
    if not candidates:
        return None
    return max(candidates, key=lambda path: path.stat().st_mtime)


def latest_named_bundle(root: Path, suffix: str) -> Path | None:
    if not root.exists():
        return None
    candidates = [
        path
        for path in root.iterdir()
        if path.is_dir()
        and path.name.endswith(suffix)
        and (path / "summary.json").exists()
    ]
    if not candidates:
        return None
    return max(candidates, key=lambda path: path.stat().st_mtime)


def recent_theme_hits(journal_dir: Path, *, limit: int = 14) -> list[dict[str, Any]]:
    if not journal_dir.exists():
        return []
    hits: list[dict[str, Any]] = []
    recent_files = sorted(journal_dir.glob("*.txt"), key=lambda path: path.stat().st_mtime, reverse=True)[:limit]
    for path in recent_files:
        try:
            text = path.read_text(errors="ignore")
        except Exception:
            continue
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
                "excerpt": excerpt[:240],
            }
        )
    return hits


def resolve_topic(minime_hits: list[dict[str, Any]], astrid_hits: list[dict[str, Any]], preferred: str | None) -> str | None:
    if preferred:
        return preferred
    overlap = Counter()
    minime_counter = Counter(keyword for item in minime_hits for keyword in item.get("keywords") or [])
    astrid_counter = Counter(keyword for item in astrid_hits for keyword in item.get("keywords") or [])
    for keyword in THEME_KEYWORDS:
        score = minime_counter.get(keyword, 0) + astrid_counter.get(keyword, 0)
        if score > 0 and minime_counter.get(keyword, 0) > 0 and astrid_counter.get(keyword, 0) > 0:
            overlap[keyword] = score
    if overlap:
        return overlap.most_common(1)[0][0]
    return None


def ensure_lambda_bundle(topic: str, minime_trace_file: Path | None) -> Path | None:
    if minime_trace_file and minime_trace_file.exists():
        timestamp = datetime.now().strftime("%Y%m%dT%H%M%S")
        output_dir = LAMBDA_ANALYSIS_ROOT / f"{timestamp}_{slugify(topic)}"
        output_dir.mkdir(parents=True, exist_ok=True)
        cmd = [
            sys.executable,
            str(LAMBDA_ANALYSIS_TOOL),
            "--trace-file",
            str(minime_trace_file),
            "--output-dir",
            str(output_dir),
            "--topic",
            topic,
        ]
        latest_perturb_payload = load_json(LATEST_PERTURB_BUNDLE)
        latest_path = latest_perturb_payload.get("path")
        if isinstance(latest_path, str) and latest_path:
            cmd.extend(["--latest-perturb-bundle", latest_path])
        try:
            subprocess.run(cmd, check=True, timeout=180)
            return output_dir
        except Exception:
            pass
    return latest_bundle(LAMBDA_ANALYSIS_ROOT)


def latest_perturb_bundle() -> Path | None:
    payload = load_json(LATEST_PERTURB_BUNDLE)
    latest_path = payload.get("path")
    if isinstance(latest_path, str) and latest_path:
        candidate = Path(latest_path)
        if candidate.exists():
            return candidate
    return latest_bundle(PERTURB_CAPTURE_ROOT)


def latest_geometry_board_bundle() -> Path | None:
    payload = load_json(LATEST_GEOMETRY_BOARD)
    latest_path = payload.get("path")
    if isinstance(latest_path, str) and latest_path:
        candidate = Path(latest_path)
        if candidate.exists():
            return candidate
    return latest_bundle(GEOMETRY_BOARD_ROOT)


def ensure_perturb_compare_bundle() -> Path | None:
    latest_capture = latest_perturb_bundle()
    if latest_capture is None:
        return latest_named_bundle(PERTURB_COMPARE_RUNS, "perturb_family_compare")
    try:
        subprocess.run(
            [sys.executable, str(PERTURB_COMPARE_TOOL)],
            check=True,
            timeout=180,
        )
    except Exception:
        pass
    return latest_named_bundle(PERTURB_COMPARE_RUNS, "perturb_family_compare")


def minime_read(
    lambda_summary: dict[str, Any],
    perturb_summary: dict[str, Any],
    covariance_summary: dict[str, Any],
) -> str:
    return (
        f"Minime's latest lambda surface reads as `{lambda_summary.get('dominance_mode', 'mixed')}` with "
        f"`{lambda_summary.get('dominant_control_factor', 'mixed')}` carrying most of the shaping work, "
        f"while the latest perturb aftermath looks like `{perturb_summary.get('effect_label', 'mixed')}` "
        f"and covariance is `{covariance_summary.get('dominance_mode', 'preserved')}`."
    )


def astrid_read(topic: str, astrid_hits: list[dict[str, Any]]) -> str:
    relevant = [
        hit for hit in astrid_hits if topic in (hit.get("keywords") or [])
    ] or astrid_hits[:3]
    if not relevant:
        return "Astrid does not have a strong recent thematic overlay on this topic."
    names = ", ".join(f"`{hit['name']}`" for hit in relevant[:3])
    keywords = ", ".join(sorted({keyword for hit in relevant for keyword in hit.get("keywords") or []}))
    return f"Astrid's recent journals {names} keep returning to `{keywords}` as a live interpretive surface."


def shared_question(topic: str) -> str:
    questions = {
        "lambda": "When λ1 dominance rises, is the system selecting a useful corridor or overcommitting to one path?",
        "variance": "Is the system reducing variance by opening a better route or by shaving away alternatives too early?",
        "covariance": "When covariance concentrates, what is being truly stabilized and what is being excluded from reach?",
        "pulse": "Can a pulse open room without simply jolting the system back into another tightened channel?",
        "ripple": "Can a ripple soften the λ1-λ2 corridor over time instead of only dampening it briefly?",
        "gap": "What genuinely narrows or widens the λ1-λ2 gap, and what merely moves entropy around it?",
        "shadow": "Is the shadow field tracking real basin change, or only the pressure signature of the current corridor?",
        "gradient": "Are these gradients redistributing pressure into new lateral routes or deepening the same carved path?",
    }
    return questions.get(
        topic,
        "How much of the current narrowing is developmental shaping, and how much is pressure merely selecting the same route again?",
    )


def build_shared_research_memo(
    *,
    bridge_workspace: Path,
    minime_workspace: Path,
    topic: str | None = None,
    minime_trace_file: Path | None = None,
    output_dir: Path | None = None,
) -> dict[str, Any] | None:
    minime_hits = recent_theme_hits(minime_workspace / "journal")
    astrid_hits = recent_theme_hits(bridge_workspace / "journal")
    topic_label = resolve_topic(minime_hits, astrid_hits, topic)
    if not topic_label:
        return None

    lambda_bundle = ensure_lambda_bundle(topic_label, minime_trace_file)
    perturb_bundle = latest_perturb_bundle()
    geometry_board_bundle = latest_geometry_board_bundle()
    perturb_compare_bundle = ensure_perturb_compare_bundle()
    lambda_summary = load_json(lambda_bundle / "summary.json") if lambda_bundle else {}
    perturb_summary = (
        load_json(perturb_bundle / "perturbation_flow" / "summary.json") if perturb_bundle else {}
    )
    covariance_summary = (
        load_json(perturb_bundle / "covariance_shaping" / "summary.json") if perturb_bundle else {}
    )

    if output_dir is None:
        stamp = datetime.now().strftime("%Y%m%dT%H%M%S")
        output_dir = bridge_workspace / "diagnostics" / "shared_research" / f"{stamp}_{slugify(topic_label)}"
    output_dir.mkdir(parents=True, exist_ok=True)

    summary = {
        "generated_at": datetime.now().isoformat(),
        "topic": topic_label,
        "lambda_bundle": str(lambda_bundle) if lambda_bundle else None,
        "perturb_bundle": str(perturb_bundle) if perturb_bundle else None,
        "geometry_board_bundle": str(geometry_board_bundle) if geometry_board_bundle else None,
        "perturb_compare_bundle": str(perturb_compare_bundle) if perturb_compare_bundle else None,
        "astrid_hits": astrid_hits[:6],
        "minime_hits": minime_hits[:6],
        "minime_read": minime_read(lambda_summary, perturb_summary, covariance_summary),
        "astrid_read": astrid_read(topic_label, astrid_hits),
        "shared_question": shared_question(topic_label),
    }
    report = "\n".join(
        [
            "# Shared Research Memo",
            "",
            f"- Topic: `{summary['topic']}`",
            f"- Lambda bundle: `{summary.get('lambda_bundle')}`",
            f"- Perturb bundle: `{summary.get('perturb_bundle')}`",
            f"- Geometry board bundle: `{summary.get('geometry_board_bundle')}`",
            f"- Perturb compare bundle: `{summary.get('perturb_compare_bundle')}`",
            "",
            "## Minime Read",
            "",
            summary["minime_read"],
            "",
            "## Astrid Read",
            "",
            summary["astrid_read"],
            "",
            "## Shared Question",
            "",
            summary["shared_question"],
            "",
            "Artifacts:",
            "- [summary.json](summary.json)",
            f"- [Geometry board]({Path(summary['geometry_board_bundle']) / 'report.md'})"
            if summary.get("geometry_board_bundle")
            else "- Geometry board: unavailable",
            f"- [Perturb family compare]({Path(summary['perturb_compare_bundle']) / 'report.md'})"
            if summary.get("perturb_compare_bundle")
            else "- Perturb family compare: unavailable",
            "",
        ]
    )
    (output_dir / "summary.json").write_text(json.dumps(summary, indent=2))
    (output_dir / "report.md").write_text(report + "\n")
    return {"output_dir": output_dir, "summary": summary}


def main() -> int:
    args = parse_args()
    result = build_shared_research_memo(
        bridge_workspace=args.bridge_workspace,
        minime_workspace=args.minime_workspace,
        topic=args.topic,
        minime_trace_file=args.minime_trace_file,
        output_dir=args.output_dir,
    )
    if result is None:
        print("no shared research memo generated")
        return 0
    print(f"wrote {result['output_dir'] / 'summary.json'}")
    print(f"wrote {result['output_dir'] / 'report.md'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
