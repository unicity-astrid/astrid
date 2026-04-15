# Review Bundles: 2026-04-02

These bundles package the live `minime` and `astrid` branch work into two
reviewable themes:

1. `control-loop`
2. `sensory-runtime`

The 12 detached Codex worktrees under `/Users/v/.codex/worktrees/*` were pruned
before creating these bundles. They were duplicate clean snapshots, not unique
lines of work.

## Patch Artifacts

- `control-loop.minime.patch`
- `control-loop.astrid.patch`
- `sensory-runtime.minime.patch`
- `sensory-runtime.astrid.patch`

All patch artifacts live in this directory.

## Suggested Review Flow

1. Read `control-loop.md`
2. Read `sensory-runtime.md`
3. Use `git apply --stat` on the patch files to inspect scope quickly
4. Review parked items only after the two main bundles are decided

## Notes

- Tracked `workspace/` churn and runtime artifact deletions were intentionally
  excluded from these bundles.
- The bundle split is thematic, not commit-backed. It is meant to make the
  current dirty branches reviewable without rewriting branch history first.
- Parked files are listed in `parked-items.md`.
