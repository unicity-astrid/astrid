"""Helpers for telemetry stability forensics reporting."""

from __future__ import annotations


def build_suspect_families(suspect_cls, parse_local_timestamp):
    return [
        suspect_cls(
            name="PI gain/runtime sovereignty layer",
            kind="code + persisted state",
            minime_commits=("2fde73a", "bb6d7b2", "13bc8ea", "839cf03"),
            astrid_commits=("58f360f8",),
            runtime_issue_ids=("sovereignty_restores_pi_gains",),
            active_from=parse_local_timestamp("2026-03-30 11:54"),
            drift_weight=18.0,
            risk_weight=12.0,
            hypothesis=(
                "Post-golden PI regime selection, self-assessment application, and "
                "later self-calibrating gain logic add extra override layers between "
                "compiled defaults and the live controller."
            ),
            next_experiment=(
                "Freeze PI gains to PIRegCfg defaults, disable sovereignty restore "
                "for pi_kp/pi_ki/pi_max_step, and rerun with the same sensory load."
            ),
        ),
        suspect_cls(
            name="Target/lambda/geom retuning",
            kind="code",
            minime_commits=("017133d", "c0831a6", "312433e", "63fb3eb"),
            astrid_commits=(),
            runtime_issue_ids=(),
            active_from=parse_local_timestamp("2026-03-29 14:09"),
            drift_weight=17.0,
            risk_weight=0.0,
            hypothesis=(
                "Post-golden retuning changed lambda/geom balance, widened clamps, "
                "and altered integrator behavior, which can weaken the controller's "
                "ability to pull fill back once recurrence is self-sustaining."
            ),
            next_experiment=(
                "Restore golden lambda/geom targets and clamp behavior together, "
                "then test against the same load before changing gains again."
            ),
        ),
        suspect_cls(
            name="Intrinsic wander / keep-floor / covariance retention changes",
            kind="code",
            minime_commits=("6a2e882", "63fb3eb"),
            astrid_commits=(),
            runtime_issue_ids=(),
            active_from=parse_local_timestamp("2026-03-30 12:22"),
            drift_weight=16.0,
            risk_weight=0.0,
            hypothesis=(
                "Extended keep_floor logic, high intrinsic_wander, and related "
                "covariance retention changes can shift the equilibrium point upward "
                "even when the nominal fill target stays fixed."
            ),
            next_experiment=(
                "Pin intrinsic_wander to 0.03, hold keep-floor/base at the golden "
                "profile, and compare 30-minute traces against the same workload."
            ),
        ),
        suspect_cls(
            name="Codec energy and normalization changes",
            kind="code",
            minime_commits=(),
            astrid_commits=("2fcc54dd", "58f360f8", "3ad60787", "46ceeb8d"),
            runtime_issue_ids=(),
            active_from=parse_local_timestamp("2026-03-29 14:34"),
            drift_weight=14.0,
            risk_weight=0.0,
            hypothesis=(
                "Lower SEMANTIC_GAIN, later codec resonance changes, and the tanh to "
                "softsign swap all change how sharply dialogue bursts drive the ESN, "
                "which can blur the burst/rest signal the PI loop sees."
            ),
            next_experiment=(
                "Revert codec gain/normalization as a bundle and compare per-exchange "
                "fill deltas before touching controller gains."
            ),
        ),
        suspect_cls(
            name="Launchd/startup/config drift",
            kind="config",
            minime_commits=(),
            astrid_commits=(),
            runtime_issue_ids=("launchd_env_plist_mismatch", "launchd_plist_missing_target"),
            active_from=parse_local_timestamp("2026-04-01 00:00"),
            drift_weight=10.0,
            risk_weight=15.0,
            hypothesis=(
                "The canonical startup path pins 0.55, but alternate launchd/restart "
                "paths can still fall back to 0.75 or inherited env state, making "
                "runtime state disagree with the intended golden-reset configuration."
            ),
            next_experiment=(
                "Use one canonical launch path only, pin EIGENFILL_TARGET in the "
                "plist and wrapper, and log the effective startup args on boot."
            ),
        ),
        suspect_cls(
            name="Newer regulation layers kept after rollback",
            kind="code + config",
            minime_commits=("839cf03", "cd85058"),
            astrid_commits=("0f9d1db1",),
            runtime_issue_ids=(),
            active_from=parse_local_timestamp("2026-04-01 10:35"),
            drift_weight=13.0,
            risk_weight=6.0,
            hypothesis=(
                "Self-calibrating PI gains, rho sovereignty, sensory-seeded noise, "
                "GOAL-driven control, and adjacent late additions can keep injecting "
                "new dynamics even after golden-period defaults are restored."
            ),
            next_experiment=(
                "Run one clean baseline with the late regulation/noise layers disabled "
                "and only the golden control surface left active."
            ),
        ),
    ]


def _commit_table_block(
    title,
    epochs,
    autonomous_events,
    *,
    local_label,
    csv_commit_label,
    intro_summary,
    summarize_autonomous,
):
    lines = [f"### {title}", ""]
    lines.append(
        "| Epoch | Duration | Avg Fill | Std Fill | Avg lambda1 | Commits | Confidence | Context |"
    )
    lines.append(
        "|-------|----------|----------|----------|-------------|---------|------------|---------|"
    )
    for epoch in epochs:
        auto_count, auto_kinds = summarize_autonomous(
            autonomous_events, epoch.epoch_start, epoch.epoch_end
        )
        commit_label = (
            f"minime `{csv_commit_label(epoch.minime_commit)}` / "
            f"astrid `{csv_commit_label(epoch.astrid_commit)}`"
        )
        lines.append(
            "| "
            f"{local_label(epoch.epoch_start)} to {local_label(epoch.epoch_end)} | "
            f"{epoch.duration_min}m | "
            f"{epoch.avg_fill:.1f}% | "
            f"{epoch.std_fill:.2f} | "
            f"{epoch.avg_lambda1:.1f} | "
            f"{commit_label} | "
            f"{epoch.confidence} | "
            f"{auto_count} autonomous msgs ({auto_kinds}) |"
        )
    lines.append("")
    for epoch in epochs:
        lines.append(f"- `{local_label(epoch.epoch_start)}`")
        lines.append(f"  minime 24h: {intro_summary(epoch.minime_intro_commits)}")
        lines.append(f"  astrid 24h: {intro_summary(epoch.astrid_intro_commits)}")
    lines.append("")
    return lines


def _validation_block(existing_hourly, generated_hourly):
    targets = [
        "2026-03-29 02:00",
        "2026-03-29 03:00",
        "2026-03-29 04:00",
        "2026-03-29 05:00",
        "2026-03-29 06:00",
        "2026-04-02 07:00",
        "2026-04-02 08:00",
        "2026-04-02 09:00",
        "2026-04-02 10:00",
        "2026-04-02 11:00",
    ]
    lines = ["## Timestamp Normalization Check", ""]
    lines.append(
        "| Hour | Existing Avg | Generated Avg | Existing lambda1 | Generated lambda1 | Delta |"
    )
    lines.append(
        "|------|--------------|---------------|------------------|-------------------|-------|"
    )
    for label in targets:
        existing = existing_hourly.get(label)
        generated = generated_hourly.get(label)
        if existing is None or generated is None:
            continue
        delta = generated.avg_fill - existing["avg_fill"]
        lines.append(
            "| "
            f"{label} | "
            f"{existing['avg_fill']:.1f}% | "
            f"{generated.avg_fill:.1f}% | "
            f"{existing['avg_lambda1']:.2f} | "
            f"{generated.avg_lambda1:.2f} | "
            f"{delta:+.3f} |"
        )
    lines.append("")
    lines.append(
        "Generated hourly buckets are labeled in America/Los_Angeles local time to "
        "match the existing `hourly_fill_summary.csv` strings."
    )
    lines.append("")
    return lines


def _healthy_hour_band_block(hourly_bands):
    lines = ["## Broader Healthy Hour Bands", ""]
    if not hourly_bands:
        lines.append("No multi-hour bands met the 62-68% hourly fill filter.")
        lines.append("")
        return lines
    lines.append("| Start | End | Hours | Avg Fill | Avg lambda1 | Notes |")
    lines.append("|-------|-----|-------|----------|-------------|-------|")
    for band in hourly_bands[:5]:
        note = (
            "Matches the historical golden period"
            if band["start"] == "2026-03-29 02:00"
            else "Supplemental healthy context"
        )
        lines.append(
            "| "
            f"{band['start']} | "
            f"{band['end']} | "
            f"{band['hours']} | "
            f"{band['avg_fill']:.1f}% | "
            f"{band['avg_lambda1']:.1f} | "
            f"{note} |"
        )
    lines.append("")
    lines.append(
        "These hourly bands are supplemental context. The strict epoch table uses the "
        "15-minute score threshold from the plan, which isolates only the calmest "
        "core of the broader March 29 healthy run."
    )
    lines.append("")
    return lines


def _suspect_block(ranked_suspects, runtime_issues, healthy_reference_pool, stuck_high_pool):
    lines = ["## Ranked Suspect Families", ""]
    lines.append(
        "| Rank | Family | Healthy Ref | Stuck-High | Kind | Why It Correlates |"
    )
    lines.append(
        "|------|--------|-------------|------------|------|-------------------|"
    )
    healthy_count = max(1, len(healthy_reference_pool))
    stuck_count = max(1, len(stuck_high_pool))
    for index, suspect in enumerate(ranked_suspects, start=1):
        family = suspect["family"]
        lines.append(
            "| "
            f"{index} | "
            f"{family.name} | "
            f"{suspect['healthy_hits']}/{healthy_count} | "
            f"{suspect['stuck_hits']}/{stuck_count} | "
            f"{family.kind} | "
            f"{family.hypothesis} |"
        )
    lines.append("")
    for index, suspect in enumerate(ranked_suspects, start=1):
        family = suspect["family"]
        intro_entries = suspect["intro_entries"]
        issue_details = suspect["issue_details"]
        lines.append(f"### {index}. {family.name}")
        if intro_entries:
            lines.append(
                "- Intro commits: "
                + "; ".join(
                    f"{entry.repo} `{entry.short_sha}` {entry.subject}"
                    for entry in intro_entries
                )
            )
        else:
            lines.append("- Intro commits: operational/config surface only")
        if issue_details:
            lines.append("- Runtime/config evidence: " + " ".join(issue_details))
        lines.append(
            f"- Correlation: present in {suspect['stuck_hits']}/{len(stuck_high_pool)} "
            f"stuck-high epochs vs {suspect['healthy_hits']}/{len(healthy_reference_pool)} "
            "healthy reference epochs."
        )
        lines.append(f"- Next test: {family.next_experiment}")
        lines.append("")

    if runtime_issues:
        lines.append("### Runtime Drift Surfaces")
        for issue in runtime_issues:
            lines.append(f"- {issue.title}: {issue.detail}")
        lines.append("")
    return lines


def _recommendation_block(ranked_suspects):
    lines = ["## Change Next", ""]
    if not ranked_suspects:
        lines.append("No suspect families were ranked.")
        lines.append("")
        return lines
    for suspect in ranked_suspects[:2]:
        lines.append(f"- {suspect['family'].next_experiment}")
    lines.append(
        "- Keep the launch path single-source-of-truth while testing so git-based "
        "attribution is not confounded by inherited env state or persisted PI gains."
    )
    lines.append("")
    return lines


def _assumptions_block(healthy_epochs):
    lines = ["## Assumptions and Caveats", ""]
    lines.append(
        "- The strict healthy-epoch table uses the exact score threshold from the plan; "
        f"that produced {len(healthy_epochs)} qualifying epoch(s) on the current DB."
    )
    lines.append(
        "- Git commit time is treated as a deploy proxy, not proof of activation. "
        "Confidence is lowered when recent commit density is high or when runtime drift "
        "surfaces are known."
    )
    lines.append(
        "- `consciousness.v1.autonomous` activity is used for context and tie-breaker "
        "notes only, not for the primary health score."
    )
    lines.append("")
    return lines


def write_report_bundle(
    path,
    *,
    existing_hourly,
    generated_hourly,
    healthy_epochs,
    healthy_reference_pool,
    stuck_high_pool,
    healthy_hour_bands,
    ranked_suspects,
    runtime_issues,
    autonomous_events,
    local_label,
    csv_commit_label,
    intro_summary,
    summarize_autonomous,
):
    lines = [
        "# Telemetry Stability Forensics",
        "",
        "Recomputed from raw `bridge.db` telemetry using 15-minute buckets, local-time "
        "labels, git-history correlation, and explicit runtime-drift penalties.",
        "",
    ]
    lines.extend(_validation_block(existing_hourly, generated_hourly))
    lines.extend(_healthy_hour_band_block(healthy_hour_bands))
    lines.extend(
        _commit_table_block(
            "Top Healthy Epochs",
            healthy_reference_pool,
            autonomous_events,
            local_label=local_label,
            csv_commit_label=csv_commit_label,
            intro_summary=intro_summary,
            summarize_autonomous=summarize_autonomous,
        )
    )
    lines.extend(
        _commit_table_block(
            "Top Stuck-High Epochs",
            stuck_high_pool,
            autonomous_events,
            local_label=local_label,
            csv_commit_label=csv_commit_label,
            intro_summary=intro_summary,
            summarize_autonomous=summarize_autonomous,
        )
    )
    lines.extend(
        _suspect_block(
            ranked_suspects,
            runtime_issues,
            healthy_reference_pool,
            stuck_high_pool,
        )
    )
    lines.extend(_recommendation_block(ranked_suspects))
    lines.extend(_assumptions_block(healthy_epochs))
    path.write_text("\n".join(lines))
