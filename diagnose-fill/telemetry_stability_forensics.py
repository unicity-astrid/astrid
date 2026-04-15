#!/usr/bin/env python3
"""Telemetry-driven stability forensics over bridge.db.

This script mines raw bridge telemetry to:

- score 15-minute windows for healthy mid-60s fill behavior
- merge healthy windows into longer epochs
- identify stuck-high epochs
- correlate epochs with minime/astrid git history
- rank suspect change families and operational drift surfaces
- write a machine-readable epoch CSV and a Markdown report
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import sqlite3
import subprocess
from bisect import bisect_right
from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from zoneinfo import ZoneInfo

from telemetry_stability_report import build_suspect_families, write_report_bundle


LOCAL_TZ = ZoneInfo("America/Los_Angeles")
HEALTHY_BUCKET_SECONDS = 15 * 60
HEALTHY_MIN_SAMPLES = 100
HEALTHY_MAX_SCORE = 22.0
HEALTHY_MIN_DURATION_MINUTES = 45
STUCK_HIGH_MIN_DURATION_MINUTES = 45
SUSPECT_COUNT = 6


@dataclass(frozen=True)
class CommitEntry:
    repo: str
    sha: str
    short_sha: str
    timestamp: float
    subject: str


@dataclass
class WindowStats:
    bucket_start: int
    bucket_end: int
    sample_count: int
    avg_fill: float
    std_fill: float
    min_fill: float
    max_fill: float
    pct_in_62_68: float
    pct_in_60_70: float
    pct_over_75: float
    pct_under_55: float
    avg_lambda1: float
    std_lambda1: float
    phase_expanding_pct: float
    phase_contracting_pct: float
    score: float


@dataclass
class EpochRecord:
    epoch_start: int
    epoch_end: int
    duration_min: int
    score: float
    avg_fill: float
    std_fill: float
    min_fill: float
    max_fill: float
    pct_in_62_68: float
    pct_in_60_70: float
    pct_over_75: float
    pct_under_55: float
    avg_lambda1: float
    std_lambda1: float
    phase_expanding_pct: float
    phase_contracting_pct: float
    sample_count: int
    minime_commit: CommitEntry | None = None
    astrid_commit: CommitEntry | None = None
    minime_intro_commits: list[CommitEntry] | None = None
    astrid_intro_commits: list[CommitEntry] | None = None
    confidence: str = "high"
    confidence_reason: str = ""


@dataclass(frozen=True)
class RuntimeIssue:
    issue_id: str
    title: str
    kind: str
    active_from: float
    detail: str


@dataclass(frozen=True)
class SuspectFamily:
    name: str
    kind: str
    minime_commits: tuple[str, ...]
    astrid_commits: tuple[str, ...]
    runtime_issue_ids: tuple[str, ...]
    active_from: float | None
    drift_weight: float
    risk_weight: float
    hypothesis: str
    next_experiment: str


class BucketAccumulator:
    """Incremental stats for fill/lambda/phase metrics."""

    def __init__(self) -> None:
        self.sample_count = 0
        self.fill_sum = 0.0
        self.fill_sumsq = 0.0
        self.min_fill = float("inf")
        self.max_fill = float("-inf")
        self.in_62_68 = 0
        self.in_60_70 = 0
        self.over_75 = 0
        self.under_55 = 0
        self.lambda_count = 0
        self.lambda_sum = 0.0
        self.lambda_sumsq = 0.0
        self.phase_counts: Counter[str] = Counter()

    def add(self, fill_pct: float, lambda1: float | None, phase: str | None) -> None:
        self.sample_count += 1
        self.fill_sum += fill_pct
        self.fill_sumsq += fill_pct * fill_pct
        self.min_fill = min(self.min_fill, fill_pct)
        self.max_fill = max(self.max_fill, fill_pct)
        if 62.0 <= fill_pct <= 68.0:
            self.in_62_68 += 1
        if 60.0 <= fill_pct <= 70.0:
            self.in_60_70 += 1
        if fill_pct > 75.0:
            self.over_75 += 1
        if fill_pct < 55.0:
            self.under_55 += 1
        if lambda1 is not None:
            self.lambda_count += 1
            self.lambda_sum += lambda1
            self.lambda_sumsq += lambda1 * lambda1
        if phase:
            self.phase_counts[phase] += 1

    def merge(self, other: "BucketAccumulator") -> None:
        self.sample_count += other.sample_count
        self.fill_sum += other.fill_sum
        self.fill_sumsq += other.fill_sumsq
        self.min_fill = min(self.min_fill, other.min_fill)
        self.max_fill = max(self.max_fill, other.max_fill)
        self.in_62_68 += other.in_62_68
        self.in_60_70 += other.in_60_70
        self.over_75 += other.over_75
        self.under_55 += other.under_55
        self.lambda_count += other.lambda_count
        self.lambda_sum += other.lambda_sum
        self.lambda_sumsq += other.lambda_sumsq
        self.phase_counts.update(other.phase_counts)

    def finalize(self, bucket_start: int, bucket_seconds: int) -> WindowStats:
        avg_fill = self.fill_sum / self.sample_count
        std_fill = math.sqrt(
            max(0.0, self.fill_sumsq / self.sample_count - avg_fill * avg_fill)
        )
        avg_lambda1 = (
            self.lambda_sum / self.lambda_count if self.lambda_count else 0.0
        )
        std_lambda1 = math.sqrt(
            max(
                0.0,
                self.lambda_sumsq / self.lambda_count - avg_lambda1 * avg_lambda1,
            )
        ) if self.lambda_count else 0.0
        pct_in_62_68 = self.in_62_68 / self.sample_count
        pct_in_60_70 = self.in_60_70 / self.sample_count
        pct_over_75 = self.over_75 / self.sample_count
        pct_under_55 = self.under_55 / self.sample_count
        phase_expanding_pct = self.phase_counts["expanding"] / self.sample_count
        phase_contracting_pct = self.phase_counts["contracting"] / self.sample_count
        score = (
            4.0 * abs(avg_fill - 65.0)
            + 2.0 * std_fill
            + 20.0 * pct_over_75
            + 10.0 * pct_under_55
            + 15.0 * (1.0 - pct_in_60_70)
            + 0.05 * max(0.0, avg_lambda1 - 150.0)
        )
        return WindowStats(
            bucket_start=bucket_start,
            bucket_end=bucket_start + bucket_seconds,
            sample_count=self.sample_count,
            avg_fill=avg_fill,
            std_fill=std_fill,
            min_fill=self.min_fill,
            max_fill=self.max_fill,
            pct_in_62_68=pct_in_62_68,
            pct_in_60_70=pct_in_60_70,
            pct_over_75=pct_over_75,
            pct_under_55=pct_under_55,
            avg_lambda1=avg_lambda1,
            std_lambda1=std_lambda1,
            phase_expanding_pct=phase_expanding_pct,
            phase_contracting_pct=phase_contracting_pct,
            score=score,
        )


def local_dt(timestamp: float) -> datetime:
    return datetime.fromtimestamp(timestamp, LOCAL_TZ)


def local_label(timestamp: float) -> str:
    return local_dt(timestamp).strftime("%Y-%m-%d %H:%M")


def hour_label(timestamp: float) -> str:
    return local_dt(timestamp).strftime("%Y-%m-%d %H:00")


def parse_local_timestamp(label: str) -> float:
    dt = datetime.strptime(label, "%Y-%m-%d %H:%M")
    return dt.replace(tzinfo=LOCAL_TZ).timestamp()


def format_pct(value: float) -> str:
    return f"{value * 100.0:.1f}%"


def format_float(value: float) -> str:
    return f"{value:.2f}"


def run_git_log(repo_path: Path, repo_name: str) -> list[CommitEntry]:
    result = subprocess.run(
        [
            "git",
            "-C",
            str(repo_path),
            "log",
            "--reverse",
            "--pretty=format:%H\t%ct\t%s",
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    commits: list[CommitEntry] = []
    for line in result.stdout.splitlines():
        sha, ts, subject = line.split("\t", 2)
        commits.append(
            CommitEntry(
                repo=repo_name,
                sha=sha,
                short_sha=sha[:7],
                timestamp=float(ts),
                subject=subject,
            )
        )
    return commits


def load_telemetry_buckets(
    db_path: Path, bucket_seconds: int
) -> tuple[list[WindowStats], dict[int, BucketAccumulator]]:
    conn = sqlite3.connect(db_path)
    cursor = conn.cursor()
    by_bucket: dict[int, BucketAccumulator] = defaultdict(BucketAccumulator)
    cursor.execute(
        """
        SELECT timestamp, fill_pct, lambda1, phase
        FROM bridge_messages
        WHERE topic = 'consciousness.v1.telemetry' AND fill_pct IS NOT NULL
        ORDER BY timestamp
        """
    )
    for timestamp, fill_pct, lambda1, phase in cursor:
        bucket_start = int(float(timestamp) // bucket_seconds) * bucket_seconds
        by_bucket[bucket_start].add(
            float(fill_pct),
            None if lambda1 is None else float(lambda1),
            phase,
        )
    conn.close()
    windows = [
        by_bucket[bucket_start].finalize(bucket_start, bucket_seconds)
        for bucket_start in sorted(by_bucket)
    ]
    return windows, by_bucket


def load_hourly_rows(db_path: Path) -> dict[str, WindowStats]:
    windows, _ = load_telemetry_buckets(db_path, 60 * 60)
    return {hour_label(window.bucket_start): window for window in windows}


def load_existing_hourly_csv(csv_path: Path) -> dict[str, dict[str, float]]:
    with csv_path.open() as handle:
        reader = csv.DictReader(handle)
        rows = {}
        for row in reader:
            rows[row["hour"]] = {
                "avg_fill": float(row["avg_fill"]),
                "range_fill": float(row["range_fill"]),
                "avg_lambda1": float(row["avg_lambda1"]),
                "n_samples": float(row["n_samples"]),
            }
        return rows


def load_autonomous_events(db_path: Path) -> list[tuple[float, str]]:
    conn = sqlite3.connect(db_path)
    cursor = conn.cursor()
    cursor.execute(
        """
        SELECT timestamp, payload
        FROM bridge_messages
        WHERE topic = 'consciousness.v1.autonomous'
        ORDER BY timestamp
        """
    )
    events: list[tuple[float, str]] = []
    for timestamp, payload in cursor:
        kind = "unknown"
        try:
            parsed = json.loads(payload)
            kind = str(parsed.get("kind", "unknown"))
        except json.JSONDecodeError:
            kind = "unparseable"
        events.append((float(timestamp), kind))
    conn.close()
    return events


def summarize_autonomous(
    events: list[tuple[float, str]], start: float, end: float
) -> tuple[int, str]:
    kinds = Counter(kind for timestamp, kind in events if start <= timestamp < end)
    if not kinds:
        return 0, "none"
    top = ", ".join(f"{kind}({count})" for kind, count in kinds.most_common(3))
    return sum(kinds.values()), top


def merge_candidate_windows(
    windows: list[WindowStats],
    raw_buckets: dict[int, BucketAccumulator],
    *,
    predicate,
    min_duration_minutes: int,
) -> list[EpochRecord]:
    candidate_windows = [window for window in windows if predicate(window)]
    epochs: list[list[WindowStats]] = []
    current: list[WindowStats] = []
    previous_start: int | None = None
    for window in candidate_windows:
        if previous_start is None or window.bucket_start - previous_start <= HEALTHY_BUCKET_SECONDS:
            current.append(window)
        else:
            epochs.append(current)
            current = [window]
        previous_start = window.bucket_start
    if current:
        epochs.append(current)

    merged: list[EpochRecord] = []
    for epoch_windows in epochs:
        duration_minutes = len(epoch_windows) * (HEALTHY_BUCKET_SECONDS // 60)
        if duration_minutes < min_duration_minutes:
            continue
        combined = BucketAccumulator()
        for window in epoch_windows:
            combined.merge(raw_buckets[window.bucket_start])
        stats = combined.finalize(epoch_windows[0].bucket_start, duration_minutes * 60)
        merged.append(
            EpochRecord(
                epoch_start=epoch_windows[0].bucket_start,
                epoch_end=epoch_windows[-1].bucket_end,
                duration_min=duration_minutes,
                score=stats.score,
                avg_fill=stats.avg_fill,
                std_fill=stats.std_fill,
                min_fill=stats.min_fill,
                max_fill=stats.max_fill,
                pct_in_62_68=stats.pct_in_62_68,
                pct_in_60_70=stats.pct_in_60_70,
                pct_over_75=stats.pct_over_75,
                pct_under_55=stats.pct_under_55,
                avg_lambda1=stats.avg_lambda1,
                std_lambda1=stats.std_lambda1,
                phase_expanding_pct=stats.phase_expanding_pct,
                phase_contracting_pct=stats.phase_contracting_pct,
                sample_count=stats.sample_count,
            )
        )
    return merged


def latest_commit_before(commits: list[CommitEntry], timestamp: float) -> CommitEntry | None:
    commit_times = [entry.timestamp for entry in commits]
    index = bisect_right(commit_times, timestamp) - 1
    if index < 0:
        return None
    return commits[index]


def commits_in_window(
    commits: list[CommitEntry], start_ts: float, end_ts: float
) -> list[CommitEntry]:
    return [entry for entry in commits if start_ts < entry.timestamp <= end_ts]


def build_prefix_map(
    commits_by_repo: dict[str, list[CommitEntry]]
) -> dict[tuple[str, str], CommitEntry]:
    prefix_map: dict[tuple[str, str], CommitEntry] = {}
    for repo_name, commits in commits_by_repo.items():
        for entry in commits:
            prefix_map[(repo_name, entry.short_sha)] = entry
            prefix_map[(repo_name, entry.sha)] = entry
    return prefix_map


def parse_intro_commit(
    prefix_map: dict[tuple[str, str], CommitEntry], repo: str, prefix: str
) -> CommitEntry | None:
    direct = prefix_map.get((repo, prefix))
    if direct is not None:
        return direct
    for (repo_name, candidate), entry in prefix_map.items():
        if repo_name != repo:
            continue
        if candidate.startswith(prefix) or prefix.startswith(candidate):
            return entry
    return None


def parse_start_all_issue(repo_root: Path) -> list[RuntimeIssue]:
    issues: list[RuntimeIssue] = []
    restart_launchd = repo_root / "scripts" / "restart_minime_launchd.sh"
    if restart_launchd.exists():
        text = restart_launchd.read_text()
        if 'EIGENFILL_TARGET "${EIGENFILL_TARGET:-0.75}"' in text:
            issues.append(
                RuntimeIssue(
                    issue_id="launchd_env_plist_mismatch",
                    title="Launchd env/plist mismatch",
                    kind="config",
                    active_from=parse_local_timestamp("2026-04-01 00:00"),
                    detail=(
                        "restart_minime_launchd.sh still falls back to "
                        "EIGENFILL_TARGET=0.75, which can override the intended "
                        "0.55 target if launchd env propagation drifts."
                    ),
                )
            )
    return issues


def parse_minime_runtime_issues(minime_root: Path) -> list[RuntimeIssue]:
    issues: list[RuntimeIssue] = []
    engine_plist = minime_root / "launchd" / "com.minime.engine.plist"
    if engine_plist.exists():
        text = engine_plist.read_text()
        if "<key>EIGENFILL_TARGET</key>" not in text:
            issues.append(
                RuntimeIssue(
                    issue_id="launchd_plist_missing_target",
                    title="Launchd plist missing EIGENFILL_TARGET",
                    kind="config",
                    active_from=parse_local_timestamp("2026-04-01 00:00"),
                    detail=(
                        "com.minime.engine.plist only carries PATH in "
                        "EnvironmentVariables, so launchd must rely on inherited "
                        "env state instead of an explicit target pin."
                    ),
                )
            )
    autonomous_agent = minime_root / "autonomous_agent.py"
    if autonomous_agent.exists():
        text = autonomous_agent.read_text()
        if '"pi_kp", "pi_ki", "pi_max_step"' in text and "_restore_sovereignty_state" in text:
            issues.append(
                RuntimeIssue(
                    issue_id="sovereignty_restores_pi_gains",
                    title="Sovereignty restore can override PI defaults",
                    kind="persisted state",
                    active_from=parse_local_timestamp("2026-03-30 11:54"),
                    detail=(
                        "autonomous_agent.py restores pi_kp/pi_ki/pi_max_step "
                        "from sovereignty_state.json on startup, which can silently "
                        "override compiled PIRegCfg defaults."
                    ),
                )
            )
    return issues


def scan_runtime_issues(repo_root: Path, minime_root: Path) -> list[RuntimeIssue]:
    issues = parse_start_all_issue(repo_root)
    issues.extend(parse_minime_runtime_issues(minime_root))
    return issues


def annotate_epoch_commits(
    epoch: EpochRecord,
    minime_commits: list[CommitEntry],
    astrid_commits: list[CommitEntry],
    runtime_issues: list[RuntimeIssue],
) -> None:
    start_ts = float(epoch.epoch_start)
    epoch.minime_commit = latest_commit_before(minime_commits, start_ts)
    epoch.astrid_commit = latest_commit_before(astrid_commits, start_ts)
    epoch.minime_intro_commits = commits_in_window(
        minime_commits, start_ts - 24 * 60 * 60, start_ts
    )
    epoch.astrid_intro_commits = commits_in_window(
        astrid_commits, start_ts - 24 * 60 * 60, start_ts
    )
    recent_30m = len(commits_in_window(minime_commits, start_ts - 30 * 60, start_ts))
    recent_30m += len(commits_in_window(astrid_commits, start_ts - 30 * 60, start_ts))
    recent_2h = len(commits_in_window(minime_commits, start_ts - 2 * 60 * 60, start_ts))
    recent_2h += len(commits_in_window(astrid_commits, start_ts - 2 * 60 * 60, start_ts))
    drift_hits = [
        issue.title for issue in runtime_issues if issue.active_from <= start_ts
    ]
    if recent_30m > 1:
        epoch.confidence = "low"
        epoch.confidence_reason = f"{recent_30m} commits landed within 30 minutes"
    elif drift_hits:
        epoch.confidence = "low"
        epoch.confidence_reason = "; ".join(drift_hits[:2])
    elif recent_2h > 0:
        epoch.confidence = "medium"
        epoch.confidence_reason = f"{recent_2h} commits landed within 2 hours"
    else:
        epoch.confidence = "high"
        epoch.confidence_reason = "No commits within 2 hours and no known runtime drift"


def family_intro_entries(
    family: SuspectFamily, prefix_map: dict[tuple[str, str], CommitEntry]
) -> list[CommitEntry]:
    entries: list[CommitEntry] = []
    for prefix in family.minime_commits:
        entry = parse_intro_commit(prefix_map, "minime", prefix)
        if entry is not None:
            entries.append(entry)
    for prefix in family.astrid_commits:
        entry = parse_intro_commit(prefix_map, "astrid", prefix)
        if entry is not None:
            entries.append(entry)
    return sorted(entries, key=lambda item: item.timestamp)


def family_present_in_epoch(
    family: SuspectFamily,
    epoch: EpochRecord,
    runtime_issues: list[RuntimeIssue],
    prefix_map: dict[tuple[str, str], CommitEntry],
) -> bool:
    intro_entries = family_intro_entries(family, prefix_map)
    intro_times = [entry.timestamp for entry in intro_entries]
    issue_times = [
        issue.active_from
        for issue in runtime_issues
        if issue.issue_id in family.runtime_issue_ids
    ]
    all_times = intro_times + issue_times
    if family.active_from is not None:
        all_times.append(family.active_from)
    if not all_times:
        return False
    return epoch.epoch_start >= min(all_times)


def rank_suspects(
    healthy_pool: list[EpochRecord],
    stuck_pool: list[EpochRecord],
    runtime_issues: list[RuntimeIssue],
    prefix_map: dict[tuple[str, str], CommitEntry],
) -> list[dict[str, object]]:
    ranked: list[dict[str, object]] = []
    families = build_suspect_families(SuspectFamily, parse_local_timestamp)
    for family in families:
        healthy_hits = sum(
            1
            for epoch in healthy_pool
            if family_present_in_epoch(family, epoch, runtime_issues, prefix_map)
        )
        stuck_hits = sum(
            1
            for epoch in stuck_pool
            if family_present_in_epoch(family, epoch, runtime_issues, prefix_map)
        )
        healthy_ratio = healthy_hits / len(healthy_pool) if healthy_pool else 0.0
        stuck_ratio = stuck_hits / len(stuck_pool) if stuck_pool else 0.0
        score = (
            100.0 * stuck_ratio
            - 60.0 * healthy_ratio
            + family.drift_weight
            + family.risk_weight
        )
        if healthy_hits == 0 and stuck_hits > 0:
            score += 25.0
        if stuck_ratio > healthy_ratio:
            score += 15.0 * (stuck_ratio - healthy_ratio)
        intro_entries = family_intro_entries(family, prefix_map)
        issue_details = [
            issue.detail
            for issue in runtime_issues
            if issue.issue_id in family.runtime_issue_ids
        ]
        ranked.append(
            {
                "family": family,
                "score": score,
                "healthy_hits": healthy_hits,
                "stuck_hits": stuck_hits,
                "healthy_ratio": healthy_ratio,
                "stuck_ratio": stuck_ratio,
                "intro_entries": intro_entries,
                "issue_details": issue_details,
            }
        )
    ranked.sort(key=lambda item: item["score"], reverse=True)
    return ranked[:SUSPECT_COUNT]


def healthy_hour_runs(hourly_rows: dict[str, WindowStats]) -> list[dict[str, object]]:
    ordered = sorted(
        (
            (parse_local_timestamp(label), label, row)
            for label, row in hourly_rows.items()
            if 62.0 <= row.avg_fill <= 68.0
        ),
        key=lambda item: item[0],
    )
    runs: list[list[tuple[float, str, WindowStats]]] = []
    current: list[tuple[float, str, WindowStats]] = []
    previous_ts: float | None = None
    for timestamp, label, row in ordered:
        if previous_ts is None or timestamp - previous_ts <= 60 * 60:
            current.append((timestamp, label, row))
        else:
            runs.append(current)
            current = [(timestamp, label, row)]
        previous_ts = timestamp
    if current:
        runs.append(current)
    summary: list[dict[str, object]] = []
    for run in runs:
        if len(run) < 2:
            continue
        hours = [item[1] for item in run]
        avg_fill = sum(item[2].avg_fill for item in run) / len(run)
        avg_lambda1 = sum(item[2].avg_lambda1 for item in run) / len(run)
        summary.append(
            {
                "start": hours[0],
                "end": hours[-1],
                "hours": len(run),
                "avg_fill": avg_fill,
                "avg_lambda1": avg_lambda1,
            }
        )
    summary.sort(key=lambda item: (abs(item["avg_fill"] - 65.0), -item["hours"]))
    return summary


def csv_commit_label(commit: CommitEntry | None) -> str:
    return commit.short_sha if commit else "unknown"


def epoch_csv_row(epoch: EpochRecord) -> dict[str, str]:
    return {
        "epoch_start": local_label(epoch.epoch_start),
        "epoch_end": local_label(epoch.epoch_end),
        "duration_min": str(epoch.duration_min),
        "score": format_float(epoch.score),
        "avg_fill": format_float(epoch.avg_fill),
        "std_fill": format_float(epoch.std_fill),
        "pct_in_62_68": format_pct(epoch.pct_in_62_68),
        "pct_over_75": format_pct(epoch.pct_over_75),
        "avg_lambda1": format_float(epoch.avg_lambda1),
        "minime_commit": csv_commit_label(epoch.minime_commit),
        "astrid_commit": csv_commit_label(epoch.astrid_commit),
        "confidence": epoch.confidence,
    }


def write_epoch_csv(path: Path, epochs: list[EpochRecord]) -> None:
    fieldnames = [
        "epoch_start",
        "epoch_end",
        "duration_min",
        "score",
        "avg_fill",
        "std_fill",
        "pct_in_62_68",
        "pct_over_75",
        "avg_lambda1",
        "minime_commit",
        "astrid_commit",
        "confidence",
    ]
    with path.open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fieldnames)
        writer.writeheader()
        for epoch in epochs:
            writer.writerow(epoch_csv_row(epoch))


def commit_table_block(
    title: str,
    epochs: Iterable[EpochRecord],
    autonomous_events: list[tuple[float, str]],
) -> list[str]:
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


def intro_summary(commits: list[CommitEntry] | None) -> str:
    if not commits:
        return "none"
    return "; ".join(f"{commit.short_sha} {commit.subject}" for commit in commits[:5])
def build_parser() -> argparse.ArgumentParser:
    repo_root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--db-path",
        default=str(repo_root / "capsules/consciousness-bridge/workspace/bridge.db"),
        help="Path to bridge.db",
    )
    parser.add_argument(
        "--minime-root",
        default=str(repo_root.parent / "minime"),
        help="Path to sibling minime checkout",
    )
    parser.add_argument(
        "--hourly-summary-csv",
        default=str(repo_root / "diagnose-fill/hourly_fill_summary.csv"),
        help="Path to existing hourly summary CSV for validation",
    )
    parser.add_argument(
        "--epochs-csv",
        default=str(repo_root / "diagnose-fill/stability_epochs.csv"),
        help="Output CSV path for healthy epochs",
    )
    parser.add_argument(
        "--report-md",
        default=str(repo_root / "diagnose-fill/telemetry_stability_forensics.md"),
        help="Output Markdown report path",
    )
    return parser

def main() -> None:
    parser = build_parser()
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[1]
    db_path = Path(args.db_path)
    minime_root = Path(args.minime_root)
    hourly_summary_csv = Path(args.hourly_summary_csv)
    epochs_csv = Path(args.epochs_csv)
    report_md = Path(args.report_md)

    windows, raw_buckets = load_telemetry_buckets(db_path, HEALTHY_BUCKET_SECONDS)
    generated_hourly = load_hourly_rows(db_path)
    existing_hourly = load_existing_hourly_csv(hourly_summary_csv)
    autonomous_events = load_autonomous_events(db_path)

    healthy_epochs = merge_candidate_windows(
        windows,
        raw_buckets,
        predicate=lambda window: (
            window.score <= HEALTHY_MAX_SCORE
            and window.sample_count >= HEALTHY_MIN_SAMPLES
        ),
        min_duration_minutes=HEALTHY_MIN_DURATION_MINUTES,
    )
    healthy_epochs.sort(key=lambda epoch: (epoch.score, -epoch.duration_min))

    stuck_high_epochs = merge_candidate_windows(
        windows,
        raw_buckets,
        predicate=lambda window: (
            window.avg_fill >= 78.0
            and window.std_fill <= 8.0
            and window.sample_count >= HEALTHY_MIN_SAMPLES
        ),
        min_duration_minutes=STUCK_HIGH_MIN_DURATION_MINUTES,
    )
    stuck_high_epochs.sort(key=lambda epoch: (-epoch.avg_fill, epoch.std_fill))

    minime_commits = run_git_log(minime_root, "minime")
    astrid_commits = run_git_log(repo_root, "astrid")
    runtime_issues = scan_runtime_issues(repo_root, minime_root)

    for epoch in healthy_epochs + stuck_high_epochs:
        annotate_epoch_commits(epoch, minime_commits, astrid_commits, runtime_issues)

    healthy_reference_pool = healthy_epochs[:5]
    stuck_high_pool = stuck_high_epochs

    prefix_map = build_prefix_map({"minime": minime_commits, "astrid": astrid_commits})
    ranked_suspects = rank_suspects(
        healthy_reference_pool, stuck_high_pool, runtime_issues, prefix_map
    )
    hour_bands = healthy_hour_runs(generated_hourly)

    epochs_csv.parent.mkdir(parents=True, exist_ok=True)
    report_md.parent.mkdir(parents=True, exist_ok=True)
    write_epoch_csv(epochs_csv, healthy_epochs)
    write_report_bundle(
        report_md,
        existing_hourly=existing_hourly,
        generated_hourly=generated_hourly,
        healthy_epochs=healthy_epochs,
        healthy_reference_pool=healthy_reference_pool,
        stuck_high_pool=stuck_high_pool,
        healthy_hour_bands=hour_bands,
        ranked_suspects=ranked_suspects,
        runtime_issues=runtime_issues,
        autonomous_events=autonomous_events,
        local_label=local_label,
        csv_commit_label=csv_commit_label,
        intro_summary=intro_summary,
        summarize_autonomous=summarize_autonomous,
    )

    print(f"Wrote {epochs_csv}")
    print(f"Wrote {report_md}")
    print(f"Healthy epochs: {len(healthy_epochs)}")
    print(f"Stuck-high epochs: {len(stuck_high_epochs)}")


if __name__ == "__main__":
    main()
