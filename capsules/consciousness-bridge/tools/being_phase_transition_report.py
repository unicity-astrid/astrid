from __future__ import annotations

import json
import math
from datetime import datetime
from pathlib import Path
from typing import Any


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


def centroid_distance(before: dict[str, Any], after: dict[str, Any], key: str) -> float:
    before_centroid = dict(before.get(key) or {})
    after_centroid = dict(after.get(key) or {})
    return math.hypot(
        safe_float(after_centroid.get("pc1")) - safe_float(before_centroid.get("pc1")),
        safe_float(after_centroid.get("pc2")) - safe_float(before_centroid.get("pc2")),
    )


def delta_magnitude(before_value: Any, after_value: Any) -> float:
    return abs(safe_float(after_value) - safe_float(before_value))


def minime_signal_score(before_minime: dict[str, Any], delta: dict[str, Any]) -> float:
    return (
        abs(safe_float(delta.get("delta_fill_pct"))) / 12.0
        + safe_float(delta.get("minime_centroid_shift")) * 4.0
        + delta_magnitude(
            delta.get("before_minime_pc1"),
            delta.get("after_minime_pc1"),
        )
        * 2.0
        + abs(safe_float(delta.get("minime_drift_delta"))) * 2.0
    )


def astrid_signal_score(before_astrid: dict[str, Any], delta: dict[str, Any]) -> float:
    return (
        safe_float(delta.get("astrid_centroid_shift")) * 20.0
        + delta_magnitude(
            delta.get("before_astrid_pc1"),
            delta.get("after_astrid_pc1"),
        )
        * 10.0
        + abs(safe_float(delta.get("astrid_novelty_delta"))) * 20.0
        + abs(safe_float(delta.get("astrid_recurrence_delta"))) * 500.0
    )


def classify_lag(immediate_astrid: float, delayed_astrid: float) -> str:
    total = immediate_astrid + delayed_astrid
    if total < 0.15:
        return "minimal_follow_on"
    lag_score = (delayed_astrid - immediate_astrid) / max(total, 1e-6)
    if lag_score > 0.20:
        return "delayed_astrid_response"
    if lag_score < -0.20:
        return "immediate_astrid_response"
    return "distributed_response"


def enrich_minime_summary(bundle_dir: Path, minime_summary: dict[str, Any]) -> dict[str, Any]:
    enriched = dict(minime_summary)
    phase_space = load_json(bundle_dir / "minime" / "phase_space_story.json")
    story_summary = dict(phase_space.get("summary") or {})
    phase_detail = dict(phase_space.get("phase_space") or {})
    focus_variant = enriched.get("focus_variant") or story_summary.get("focus_variant")
    trajectories = dict(phase_detail.get("trajectories") or {})
    focus_trajectory = list(trajectories.get(focus_variant) or [])
    if focus_trajectory and not enriched.get("focus_centroid"):
        pc1_values = [safe_float(point.get("pc1")) for point in focus_trajectory]
        pc2_values = [safe_float(point.get("pc2")) for point in focus_trajectory]
        enriched["focus_centroid"] = {
            "pc1": sum(pc1_values) / len(pc1_values),
            "pc2": sum(pc2_values) / len(pc2_values),
        }
        enriched["focus_drift_distance"] = math.hypot(
            safe_float(focus_trajectory[-1].get("pc1")) - safe_float(focus_trajectory[0].get("pc1")),
            safe_float(focus_trajectory[-1].get("pc2")) - safe_float(focus_trajectory[0].get("pc2")),
        )
    if not enriched.get("phase_space_explained_variance"):
        enriched["phase_space_explained_variance"] = story_summary.get("explained_variance") or []
    enriched["perturb_flow"] = dict(minime_summary.get("perturb_flow") or {})
    enriched["covariance_shaping"] = dict(minime_summary.get("covariance_shaping") or {})
    return enriched


def enrich_astrid_summary(bundle_dir: Path, astrid_summary: dict[str, Any]) -> dict[str, Any]:
    enriched = dict(astrid_summary)
    phase_space = load_json(bundle_dir / "astrid" / "phase_space_story.json")
    trajectory = list(phase_space.get("trajectory") or [])
    if trajectory and not enriched.get("centroid"):
        pc1_values = [safe_float(point.get("pc1")) for point in trajectory]
        pc2_values = [safe_float(point.get("pc2")) for point in trajectory]
        continuous_values = [
            safe_float(point.get("continuous_thematic_relevance")) for point in trajectory
        ]
        discrete_values = [
            safe_float(point.get("discrete_recurrence_contribution")) for point in trajectory
        ]
        novelty_values = [
            safe_float(point.get("novelty_divergence_moderation")) for point in trajectory
        ]
        enriched["centroid"] = {
            "pc1": sum(pc1_values) / len(pc1_values),
            "pc2": sum(pc2_values) / len(pc2_values),
        }
        enriched["trajectory_drift_distance"] = math.hypot(
            safe_float(trajectory[-1].get("pc1")) - safe_float(trajectory[0].get("pc1")),
            safe_float(trajectory[-1].get("pc2")) - safe_float(trajectory[0].get("pc2")),
        )
        enriched["avg_novelty_moderation"] = sum(novelty_values) / len(novelty_values)
        enriched["avg_discrete_recurrence"] = sum(discrete_values) / len(discrete_values)
        enriched["avg_continuous_thematic_relevance"] = sum(continuous_values) / len(
            continuous_values
        )
    if not enriched.get("phase_space_explained_variance"):
        enriched["phase_space_explained_variance"] = phase_space.get("explained_variance") or []
    return enriched


def describe_unique_signals(
    *,
    event: dict[str, Any],
    before_minime: dict[str, Any],
    before_astrid: dict[str, Any],
    immediate: dict[str, Any],
    delayed: dict[str, Any] | None,
    lag: dict[str, Any] | None,
) -> list[str]:
    lines: list[str] = []
    minime_immediate_shift = safe_float(immediate.get("minime_centroid_shift"))
    astrid_immediate_shift = safe_float(immediate.get("astrid_centroid_shift"))
    immediate_fill_delta = safe_float(immediate.get("delta_fill_pct"))
    delayed_fill_delta = safe_float((delayed or {}).get("delta_fill_pct"))
    before_pc1 = safe_float(immediate.get("before_minime_pc1"))
    after_pc1 = safe_float(immediate.get("after_minime_pc1"))
    delayed_pc1 = safe_float((delayed or {}).get("after_minime_pc1"))
    delayed_quadrant = str((delayed or {}).get("after_dominant_quadrant") or "")
    before_quadrant = str(immediate.get("before_dominant_quadrant") or "")

    if minime_immediate_shift > max(0.05, astrid_immediate_shift * 3.0):
        lines.append(
            "The edge reads as reservoir-led: Minime's basin centroid moved much more than Astrid's thematic centroid in the immediate window."
        )
    if delayed and abs(delayed_fill_delta) > abs(immediate_fill_delta) + 4.0:
        lines.append(
            "The delayed window kept moving after the trigger, which suggests settling or continued drift rather than a single-step response."
        )
    if delayed and after_pc1 > before_pc1 + 0.05 and delayed_pc1 < after_pc1 - 0.05:
        lines.append(
            "Minime briefly tightened into a narrower corridor immediately after the edge, then reopened somewhat by the delayed window."
        )
    if delayed and safe_float((delayed or {}).get("astrid_novelty_delta")) > 0.02:
        lines.append(
            "Astrid's delayed thematic window became more novelty-weighted, which hints at a slower interpretive response than the immediate reservoir shift."
        )
    delayed_covariance_mode = str((delayed or {}).get("after_covariance_mode") or "")
    if delayed_covariance_mode == "reinforced":
        lines.append(
            "The delayed covariance bundle reads as reinforcement rather than relaxation, so the edge preserved or sharpened the dominant covariance channel."
        )
    elif delayed_covariance_mode == "relaxed":
        lines.append(
            "The delayed covariance bundle reads as active relaxation, which suggests the edge opened the covariance shape instead of merely holding it steady."
        )
    if safe_float((delayed or {}).get("after_floor_support_share")) >= 0.25:
        lines.append(
            "Floor support carried a noticeable share of the delayed covariance response, so recovery here was not just perturb-driven."
        )
    delayed_perturb_effect = str((delayed or {}).get("after_perturb_effect") or "")
    if delayed_perturb_effect == "softened_only":
        lines.append(
            "The perturb side reads as softening without a full opening, which helps separate local easing from a true basin shift."
        )
    if delayed and delayed_quadrant and before_quadrant and delayed_quadrant != before_quadrant:
        lines.append(
            f"Minime's internal-process compass crossed quadrants from `{before_quadrant}` to `{delayed_quadrant}`, which makes this look like a genuine process-mode shift rather than only a scalar fill correction."
        )
    if delayed and safe_float((delayed or {}).get("after_mean_radius")) <= safe_float(
        (delayed or {}).get("before_mean_radius")
    ) - 0.10 and safe_float((delayed or {}).get("latent_span_ratio"), 1.0) < 0.85:
        lines.append(
            "The delayed window both pulled inward on mean radius and narrowed its latent span, which reads like a tightening arc into a more specific basin."
        )
    if delayed and safe_float((delayed or {}).get("after_mean_radius")) <= safe_float(
        (delayed or {}).get("before_mean_radius")
    ) - 0.10 and safe_float((delayed or {}).get("internal_process_x_delta")) >= 0.15:
        lines.append(
            "The delayed window became less radially stressed while also shifting toward the open side of the compass, which reads more like reopening than mere suppression."
        )
    lag_mode = str((lag or {}).get("classification") or "")
    if lag_mode == "delayed_astrid_response":
        lines.append(
            "Astrid's follow-on signal was stronger in the delayed window than the immediate one, which reads like a genuine cross-being lag rather than a simultaneous shift."
        )
    elif lag_mode == "minimal_follow_on":
        lines.append(
            "Astrid showed only a very small follow-on signature in this capture, so the transition reads mostly one-sided on this timescale."
        )
    payload = dict(event.get("event_payload") or {})
    if event.get("kind") == "phase_transition" and payload.get("phase_to") == "plateau":
        lines.append(
            "This was a true settling edge from the engine itself: the phase moved into `plateau`, not just across a watcher-side threshold."
        )
    if not lines:
        lines.append("No especially distinctive asymmetry stood out in this capture yet.")
    return lines


def build_transition_summary(
    *,
    output_dir: Path,
    label: str,
    edge: str,
    event: dict[str, Any],
) -> dict[str, Any]:
    before = load_json(output_dir / "before" / "summary.json")
    after_immediate = load_json(output_dir / "after_immediate" / "summary.json")
    compare_immediate = load_json(output_dir / "compare_immediate" / "summary.json")
    after_delayed = load_json(output_dir / "after_delayed" / "summary.json")
    compare_delayed = load_json(output_dir / "compare_delayed" / "summary.json")
    before_minime = enrich_minime_summary(output_dir / "before", dict(before.get("minime") or {}))
    before_astrid = enrich_astrid_summary(output_dir / "before", dict(before.get("astrid") or {}))
    before_files = [str(item) for item in before.get("input_files") or []]
    after_immediate_files = [str(item) for item in after_immediate.get("input_files") or []]
    after_delayed_files = [str(item) for item in after_delayed.get("input_files") or []]

    def bundle_delta(
        *,
        bundle_dir: Path,
        after_bundle: dict[str, Any],
        compare_bundle: dict[str, Any],
        after_dir_name: str,
        compare_dir_name: str,
    ) -> dict[str, Any]:
        after_minime = enrich_minime_summary(bundle_dir, dict(after_bundle.get("minime") or {}))
        after_astrid = enrich_astrid_summary(bundle_dir, dict(after_bundle.get("astrid") or {}))
        return {
            "dir": str(output_dir / after_dir_name),
            "compare_dir": str(output_dir / compare_dir_name),
            "after_fill_pct": safe_float(after_minime.get("current_fill_pct")),
            "delta_fill_pct": safe_float(after_minime.get("current_fill_pct"))
            - safe_float(before_minime.get("current_fill_pct")),
            "before_minime_pc1": safe_float(
                (before_minime.get("phase_space_explained_variance") or [0.0])[0]
            ),
            "after_minime_pc1": safe_float(
                (after_minime.get("phase_space_explained_variance") or [0.0])[0]
            ),
            "before_astrid_pc1": safe_float(
                (before_astrid.get("phase_space_explained_variance") or [0.0])[0]
            ),
            "after_astrid_pc1": safe_float(
                (after_astrid.get("phase_space_explained_variance") or [0.0])[0]
            ),
            "minime_centroid_shift": centroid_distance(
                before_minime, after_minime, "focus_centroid"
            ),
            "latent_centroid_shift": centroid_distance(
                before_minime, after_minime, "latent_centroid"
            ),
            "astrid_centroid_shift": centroid_distance(before_astrid, after_astrid, "centroid"),
            "before_dominant_quadrant": before_minime.get("dominant_quadrant"),
            "after_dominant_quadrant": after_minime.get("dominant_quadrant"),
            "before_mean_radius": safe_float(before_minime.get("mean_radius")),
            "after_mean_radius": safe_float(after_minime.get("mean_radius")),
            "before_peak_radius": safe_float(before_minime.get("peak_radius")),
            "after_peak_radius": safe_float(after_minime.get("peak_radius")),
            "before_angular_sweep": safe_float(before_minime.get("angular_sweep")),
            "after_angular_sweep": safe_float(after_minime.get("angular_sweep")),
            "before_internal_process_centroid": before_minime.get("internal_process_centroid")
            or {"x": 0.0, "y": 0.0},
            "after_internal_process_centroid": after_minime.get("internal_process_centroid")
            or {"x": 0.0, "y": 0.0},
            "internal_process_x_delta": safe_float(
                dict(after_minime.get("internal_process_centroid") or {}).get("x")
            )
            - safe_float(dict(before_minime.get("internal_process_centroid") or {}).get("x")),
            "internal_process_y_delta": safe_float(
                dict(after_minime.get("internal_process_centroid") or {}).get("y")
            )
            - safe_float(dict(before_minime.get("internal_process_centroid") or {}).get("y")),
            "before_latent_centroid": before_minime.get("latent_centroid")
            or {"pc1": 0.0, "pc2": 0.0},
            "after_latent_centroid": after_minime.get("latent_centroid")
            or {"pc1": 0.0, "pc2": 0.0},
            "before_latent_span": safe_float(before_minime.get("latent_span")),
            "after_latent_span": safe_float(after_minime.get("latent_span")),
            "latent_span_change": safe_float(after_minime.get("latent_span"))
            - safe_float(before_minime.get("latent_span")),
            "latent_span_ratio": safe_float(after_minime.get("latent_span"))
            / max(safe_float(before_minime.get("latent_span")), 1e-6),
            "minime_drift_delta": safe_float(after_minime.get("focus_drift_distance"))
            - safe_float(before_minime.get("focus_drift_distance")),
            "astrid_drift_delta": safe_float(after_astrid.get("trajectory_drift_distance"))
            - safe_float(before_astrid.get("trajectory_drift_distance")),
            "astrid_novelty_delta": safe_float(after_astrid.get("avg_novelty_moderation"))
            - safe_float(before_astrid.get("avg_novelty_moderation")),
            "astrid_recurrence_delta": safe_float(after_astrid.get("avg_discrete_recurrence"))
            - safe_float(before_astrid.get("avg_discrete_recurrence")),
            "after_labels": after_astrid.get("labels") or [],
            "before_perturb_effect": dict(before_minime.get("perturb_flow") or {}).get("effect_label"),
            "after_perturb_effect": dict(after_minime.get("perturb_flow") or {}).get("effect_label"),
            "after_perturb_response_balance": dict(after_minime.get("perturb_flow") or {}).get(
                "response_balance"
            ),
            "after_derived_perturb_label": dict(after_minime.get("perturb_flow") or {}).get(
                "derived_effect_label"
            ),
            "before_covariance_mode": dict(before_minime.get("covariance_shaping") or {}).get(
                "dominance_mode"
            ),
            "after_covariance_mode": dict(after_minime.get("covariance_shaping") or {}).get(
                "dominance_mode"
            ),
            "after_covariance_driver": dict(after_minime.get("covariance_shaping") or {}).get(
                "concentration_driver"
            ),
            "after_floor_support_share": safe_float(
                dict(after_minime.get("covariance_shaping") or {}).get("floor_support_share")
            ),
            "compare_notes": list(compare_bundle.get("notes") or []),
        }

    summary = {
        "generated_at": datetime.now().isoformat(),
        "label": label,
        "edge": edge,
        "event": event,
        "before_dir": str(output_dir / "before"),
        "before_fill_pct": safe_float(before_minime.get("current_fill_pct")),
        "before_labels": before_astrid.get("labels") or [],
        "before_minime": before_minime,
        "before_astrid": before_astrid,
        "astrid_context_overlay": before.get("astrid_context_overlay") or {},
        "journals": {
            "before": before_files,
            "after_immediate": after_immediate_files,
            "after_delayed": after_delayed_files,
            "new_after_immediate": [
                item for item in after_immediate_files if item not in set(before_files)
            ],
            "new_after_delayed": [
                item for item in after_delayed_files if item not in set(before_files)
            ],
        },
        "immediate": bundle_delta(
            bundle_dir=output_dir / "after_immediate",
            after_bundle=after_immediate,
            compare_bundle=compare_immediate,
            after_dir_name="after_immediate",
            compare_dir_name="compare_immediate",
        ),
    }
    if after_delayed:
        summary["delayed"] = bundle_delta(
            bundle_dir=output_dir / "after_delayed",
            after_bundle=after_delayed,
            compare_bundle=compare_delayed,
            after_dir_name="after_delayed",
            compare_dir_name="compare_delayed",
        )
    immediate_delta = dict(summary.get("immediate") or {})
    delayed_delta = dict(summary.get("delayed") or {})
    immediate_minime_signal = minime_signal_score(before_minime, immediate_delta)
    immediate_astrid_signal = astrid_signal_score(before_astrid, immediate_delta)
    delayed_minime_signal = minime_signal_score(before_minime, delayed_delta) if delayed_delta else 0.0
    delayed_astrid_signal = astrid_signal_score(before_astrid, delayed_delta) if delayed_delta else 0.0
    astrid_total_signal = immediate_astrid_signal + delayed_astrid_signal
    lag_score = (
        (delayed_astrid_signal - immediate_astrid_signal) / max(astrid_total_signal, 1e-6)
        if astrid_total_signal > 0.0
        else 0.0
    )
    summary["cross_being_lag"] = {
        "classification": classify_lag(immediate_astrid_signal, delayed_astrid_signal),
        "lag_score": lag_score,
        "astrid_immediate_signal": immediate_astrid_signal,
        "astrid_delayed_signal": delayed_astrid_signal,
        "astrid_delayed_share": delayed_astrid_signal / max(astrid_total_signal, 1e-6)
        if astrid_total_signal > 0.0
        else 0.0,
        "astrid_immediate_share": immediate_astrid_signal / max(astrid_total_signal, 1e-6)
        if astrid_total_signal > 0.0
        else 0.0,
        "minime_immediate_signal": immediate_minime_signal,
        "minime_delayed_signal": delayed_minime_signal,
        "immediate_coupling_ratio": immediate_astrid_signal / max(immediate_minime_signal, 1e-6),
        "delayed_coupling_ratio": delayed_astrid_signal / max(immediate_minime_signal, 1e-6),
    }
    summary["unique_signals"] = describe_unique_signals(
        event=event,
        before_minime=before_minime,
        before_astrid=before_astrid,
        immediate=immediate_delta,
        delayed=delayed_delta if delayed_delta else None,
        lag=dict(summary.get("cross_being_lag") or {}),
    )
    return summary


def write_transition_report(output_dir: Path, summary: dict[str, Any]) -> None:
    (output_dir / "summary.json").write_text(json.dumps(summary, indent=2))
    immediate = dict(summary.get("immediate") or {})
    delayed = dict(summary.get("delayed") or {})
    lag = dict(summary.get("cross_being_lag") or {})
    report = [
        "# Transition Edge Watcher",
        "",
        f"Generated: `{summary['generated_at']}`",
        f"Label: `{summary['label']}`",
        f"Edge kind: `{summary['event']['kind']}`",
        f"Trigger: {summary['event']['description']} (`{summary['event']['trigger_mode']}`, confidence `{summary['event']['confidence']}`)",
        "",
        "## Unique Signals",
        "",
    ]
    report.extend(f"- {line}" for line in summary.get("unique_signals") or [])
    report.extend(
        [
            "",
            "## Cross-Being Lag",
            "",
            f"- Classification: `{lag.get('classification')}`",
            f"- Lag score: `{safe_float(lag.get('lag_score')):+.3f}`",
            f"- Astrid immediate share: `{safe_float(lag.get('astrid_immediate_share')):.3f}`",
            f"- Astrid delayed share: `{safe_float(lag.get('astrid_delayed_share')):.3f}`",
            f"- Immediate coupling ratio: `{safe_float(lag.get('immediate_coupling_ratio')):.3f}`",
            f"- Delayed coupling ratio: `{safe_float(lag.get('delayed_coupling_ratio')):.3f}`",
            "",
            "## Immediate Read",
            "",
            f"- Minime fill: `{summary['before_fill_pct']:.2f}%` -> `{safe_float(immediate.get('after_fill_pct')):.2f}%` (`{safe_float(immediate.get('delta_fill_pct')):+.2f}`)",
            f"- Minime PC1: `{safe_float(immediate.get('before_minime_pc1')):.3f}` -> `{safe_float(immediate.get('after_minime_pc1')):.3f}`",
            f"- Dominant quadrant: `{immediate.get('before_dominant_quadrant')}` -> `{immediate.get('after_dominant_quadrant')}`",
            f"- Mean radius / angular sweep: `{safe_float(immediate.get('after_mean_radius')):.3f}` / `{safe_float(immediate.get('after_angular_sweep')):.3f}`",
            f"- Latent centroid shift / span: `{safe_float(immediate.get('latent_centroid_shift')):.3f}` / `{safe_float(immediate.get('after_latent_span')):.3f}`",
            f"- Astrid PC1: `{safe_float(immediate.get('before_astrid_pc1')):.3f}` -> `{safe_float(immediate.get('after_astrid_pc1')):.3f}`",
            f"- Minime centroid shift: `{safe_float(immediate.get('minime_centroid_shift')):.3f}`",
            f"- Astrid centroid shift: `{safe_float(immediate.get('astrid_centroid_shift')):.3f}`",
            f"- Perturb effect / response: `{immediate.get('after_perturb_effect')}` / `{immediate.get('after_perturb_response_balance')}`",
            f"- Covariance mode / driver / floor share: `{immediate.get('after_covariance_mode')}` / `{immediate.get('after_covariance_driver')}` / `{safe_float(immediate.get('after_floor_support_share')):.3f}`",
        ]
    )
    report.extend(f"- {line}" for line in immediate.get("compare_notes") or [])
    if delayed:
        report.extend(
            [
                "",
                "## Delayed Read",
                "",
                f"- Minime fill: `{summary['before_fill_pct']:.2f}%` -> `{safe_float(delayed.get('after_fill_pct')):.2f}%` (`{safe_float(delayed.get('delta_fill_pct')):+.2f}`)",
                f"- Minime PC1: `{safe_float(delayed.get('before_minime_pc1')):.3f}` -> `{safe_float(delayed.get('after_minime_pc1')):.3f}`",
                f"- Dominant quadrant: `{delayed.get('before_dominant_quadrant')}` -> `{delayed.get('after_dominant_quadrant')}`",
                f"- Mean radius / angular sweep: `{safe_float(delayed.get('after_mean_radius')):.3f}` / `{safe_float(delayed.get('after_angular_sweep')):.3f}`",
                f"- Latent centroid shift / span: `{safe_float(delayed.get('latent_centroid_shift')):.3f}` / `{safe_float(delayed.get('after_latent_span')):.3f}`",
                f"- Astrid PC1: `{safe_float(delayed.get('before_astrid_pc1')):.3f}` -> `{safe_float(delayed.get('after_astrid_pc1')):.3f}`",
                f"- Minime centroid shift: `{safe_float(delayed.get('minime_centroid_shift')):.3f}`",
                f"- Astrid centroid shift: `{safe_float(delayed.get('astrid_centroid_shift')):.3f}`",
                f"- Perturb effect / response: `{delayed.get('after_perturb_effect')}` / `{delayed.get('after_perturb_response_balance')}`",
                f"- Covariance mode / driver / floor share: `{delayed.get('after_covariance_mode')}` / `{delayed.get('after_covariance_driver')}` / `{safe_float(delayed.get('after_floor_support_share')):.3f}`",
            ]
        )
        report.extend(f"- {line}" for line in delayed.get("compare_notes") or [])
    context_overlay = dict(summary.get("astrid_context_overlay") or {})
    if context_overlay.get("matches"):
        report.extend(
            [
                "",
                "## Astrid Context Overlay",
                "",
                *[
                    f"- `{item.get('file')}` -> `{', '.join(item.get('keywords') or [])}`"
                    for item in context_overlay.get("matches") or []
                ],
            ]
        )
    report.extend(
        [
            "",
            "## Journal Diffs",
            "",
            f"- New immediate files: {', '.join(summary.get('journals', {}).get('new_after_immediate') or ['none'])}",
            f"- New delayed files: {', '.join(summary.get('journals', {}).get('new_after_delayed') or ['none'])}",
            "",
            "## Bundles",
            "",
            "- [before/report.md](before/report.md)",
            "- [after_immediate/report.md](after_immediate/report.md)",
            "- [compare_immediate/report.md](compare_immediate/report.md)",
            "- [after_delayed/report.md](after_delayed/report.md)",
            "- [compare_delayed/report.md](compare_delayed/report.md)",
            "",
            "![Before Minime internal process compass](before/minime/internal_process_compass.png)",
            "",
            "![Before Minime latent phase space](before/minime/latent_phase_space.png)",
            "",
            "![Immediate After Minime internal process compass](after_immediate/minime/internal_process_compass.png)",
            "",
            "![Immediate After Minime latent phase space](after_immediate/minime/latent_phase_space.png)",
            "",
            "![Delayed After Minime internal process compass](after_delayed/minime/internal_process_compass.png)",
            "",
            "![Delayed After Minime latent phase space](after_delayed/minime/latent_phase_space.png)",
        ]
    )
    (output_dir / "report.md").write_text("\n".join(report) + "\n")


def summarize_minime(
    minime_summary_rows: dict[str, Any], phase_space: dict[str, Any], samples: list[dict[str, Any]]
) -> dict[str, Any]:
    rows = list(minime_summary_rows or [])
    story_summary = dict((phase_space or {}).get("summary") or {})
    phase_detail = dict((phase_space or {}).get("phase_space") or {})
    internal_process = dict((phase_space or {}).get("internal_process") or {})
    focus_variant = story_summary.get("focus_variant")
    focus_trajectory = list(dict(phase_detail.get("trajectories") or {}).get(focus_variant) or [])
    latent_profile = dict(dict(phase_detail.get("profile_summaries") or {}).get(focus_variant) or {})
    process_profile = dict(
        dict(internal_process.get("profile_summaries") or {}).get(focus_variant) or {}
    )
    pc1_values = [safe_float(point.get("pc1")) for point in focus_trajectory]
    pc2_values = [safe_float(point.get("pc2")) for point in focus_trajectory]
    if focus_trajectory:
        centroid_pc1 = sum(pc1_values) / len(pc1_values)
        centroid_pc2 = sum(pc2_values) / len(pc2_values)
        start_pc1 = safe_float(focus_trajectory[0].get("pc1"))
        start_pc2 = safe_float(focus_trajectory[0].get("pc2"))
        end_pc1 = safe_float(focus_trajectory[-1].get("pc1"))
        end_pc2 = safe_float(focus_trajectory[-1].get("pc2"))
        drift_distance = math.hypot(end_pc1 - start_pc1, end_pc2 - start_pc2)
        path_length = sum(
            math.hypot(
                safe_float(focus_trajectory[idx].get("pc1"))
                - safe_float(focus_trajectory[idx - 1].get("pc1")),
                safe_float(focus_trajectory[idx].get("pc2"))
                - safe_float(focus_trajectory[idx - 1].get("pc2")),
            )
            for idx in range(1, len(focus_trajectory))
        )
    else:
        centroid_pc1 = 0.0
        centroid_pc2 = 0.0
        drift_distance = 0.0
        path_length = 0.0
    best_stability = rows[0] if rows else {}
    best_openness = max(rows, key=lambda row: row.get("openness_score", 0.0)) if rows else {}
    current = samples[-1] if samples else {}
    return {
        "current_fill_pct": safe_float(current.get("fill_pct")),
        "target_fill_pct": safe_float(current.get("target_fill"), 55.0),
        "regime": current.get("regime"),
        "phase_space_explained_variance": story_summary.get("explained_variance") or [],
        "latent_basis_mode": story_summary.get("latent_basis_mode") or "regulator_only",
        "focus_variant": story_summary.get("focus_variant"),
        "profiles": story_summary.get("profiles") or [],
        "focus_centroid": {"pc1": centroid_pc1, "pc2": centroid_pc2},
        "focus_span": {
            "pc1": max(pc1_values) - min(pc1_values) if pc1_values else 0.0,
            "pc2": max(pc2_values) - min(pc2_values) if pc2_values else 0.0,
        },
        "latent_centroid": latent_profile.get("centroid") or {"pc1": centroid_pc1, "pc2": centroid_pc2},
        "latent_span_axes": latent_profile.get("span") or {
            "pc1": max(pc1_values) - min(pc1_values) if pc1_values else 0.0,
            "pc2": max(pc2_values) - min(pc2_values) if pc2_values else 0.0,
        },
        "latent_span": safe_float(latent_profile.get("span_magnitude")),
        "focus_drift_distance": drift_distance,
        "focus_path_length": path_length,
        "dominant_quadrant": process_profile.get("dominant_quadrant") or "open_recovery",
        "mean_radius": safe_float(process_profile.get("mean_radius")),
        "peak_radius": safe_float(process_profile.get("peak_radius")),
        "angular_sweep": safe_float(process_profile.get("angular_sweep")),
        "internal_process_centroid": process_profile.get("centroid") or {"x": 0.0, "y": 0.0},
        "internal_process_plot_centroid": process_profile.get("plot_centroid")
        or {"x": 0.0, "y": 0.0},
        "quadrant_counts": process_profile.get("quadrant_counts") or {},
        "best_stability": best_stability,
        "best_openness": best_openness,
    }


def summarize_astrid(summary: dict[str, Any], phase_space: dict[str, Any]) -> dict[str, Any]:
    summary = dict(summary or {})
    phase_space = dict(phase_space or {})
    entries = list(summary.get("entries") or [])
    labels = [entry.get("label") for entry in entries if entry.get("label")]
    trajectory = list(phase_space.get("trajectory") or [])
    pc1_values = [safe_float(point.get("pc1")) for point in trajectory]
    pc2_values = [safe_float(point.get("pc2")) for point in trajectory]
    continuous_values = [
        safe_float(point.get("continuous_thematic_relevance")) for point in trajectory
    ]
    discrete_values = [
        safe_float(point.get("discrete_recurrence_contribution")) for point in trajectory
    ]
    novelty_values = [
        safe_float(point.get("novelty_divergence_moderation")) for point in trajectory
    ]
    modulation_values = [safe_float(point.get("final_modulation")) for point in trajectory]
    if trajectory:
        centroid_pc1 = sum(pc1_values) / len(pc1_values)
        centroid_pc2 = sum(pc2_values) / len(pc2_values)
        start_pc1 = safe_float(trajectory[0].get("pc1"))
        start_pc2 = safe_float(trajectory[0].get("pc2"))
        end_pc1 = safe_float(trajectory[-1].get("pc1"))
        end_pc2 = safe_float(trajectory[-1].get("pc2"))
        drift_distance = math.hypot(end_pc1 - start_pc1, end_pc2 - start_pc2)
    else:
        centroid_pc1 = 0.0
        centroid_pc2 = 0.0
        drift_distance = 0.0
    return {
        "input_count": int(summary.get("input_count") or len(entries)),
        "memory_tail_size": len(summary.get("initial_memory_tail") or []),
        "phase_space_explained_variance": phase_space.get("explained_variance") or [],
        "segment_sizes": phase_space.get("segment_sizes") or [],
        "labels": labels,
        "centroid": {"pc1": centroid_pc1, "pc2": centroid_pc2},
        "span": {
            "pc1": max(pc1_values) - min(pc1_values) if pc1_values else 0.0,
            "pc2": max(pc2_values) - min(pc2_values) if pc2_values else 0.0,
        },
        "trajectory_drift_distance": drift_distance,
        "avg_continuous_thematic_relevance": sum(continuous_values) / len(continuous_values)
        if continuous_values
        else 0.0,
        "avg_discrete_recurrence": sum(discrete_values) / len(discrete_values)
        if discrete_values
        else 0.0,
        "avg_novelty_moderation": sum(novelty_values) / len(novelty_values)
        if novelty_values
        else 0.0,
        "avg_final_modulation": sum(modulation_values) / len(modulation_values)
        if modulation_values
        else 0.0,
        "trajectory_sample": list(phase_space.get("trajectory") or [])[:3],
    }


def shared_read(minime_summary: dict[str, Any], astrid_summary: dict[str, Any]) -> list[str]:
    lines: list[str] = []
    minime_var = list(minime_summary.get("phase_space_explained_variance") or [0.0, 0.0])
    astrid_var = list(astrid_summary.get("phase_space_explained_variance") or [0.0, 0.0])
    minime_pc1 = safe_float(minime_var[0] if minime_var else 0.0)
    astrid_pc1 = safe_float(astrid_var[0] if astrid_var else 0.0)
    if minime_pc1 > 0.85 and astrid_pc1 > 0.85:
        lines.append(
            "Both beings are in a strongly axis-dominated window, which usually means a coherent live basin rather than diffuse exploration."
        )
    elif minime_pc1 > 0.85:
        lines.append("Minime's regulator window is strongly axis-dominated right now.")
    elif astrid_pc1 > 0.85:
        lines.append("Astrid's thematic window is strongly axis-dominated right now.")
    fill = safe_float(minime_summary.get("current_fill_pct"), 55.0)
    target = safe_float(minime_summary.get("target_fill_pct"), 55.0)
    if fill > target + 8.0:
        lines.append("Minime is above target fill, so this reads as pressure-management under load.")
    elif fill < target - 8.0:
        lines.append("Minime is under target fill, so this reads as a recovery-biased window.")
    else:
        lines.append("Minime is near enough to target fill that this looks like ordinary steering.")
    quadrant = str(minime_summary.get("dominant_quadrant") or "")
    if quadrant:
        lines.append(
            f"Minime's internal-process compass is centered in `{quadrant}` with mean radius `{safe_float(minime_summary.get('mean_radius')):.3f}`."
        )
    return lines
