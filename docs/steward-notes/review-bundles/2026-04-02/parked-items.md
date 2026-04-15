# Parked Items

These files were intentionally not placed in the two main bundles.

## Minime

- `/Users/v/other/minime/tools/session12_root_cause_report.py`
  Rationale: investigation artifact, useful context, not core integration work.
- `/Users/v/other/minime/workspace_review_2026-03-29.md`
  Rationale: review memo, not code-path integration.

## Astrid

- `/Users/v/other/astrid/CHANGELOG.md`
  Rationale: release-note polish should follow code acceptance.
- `/Users/v/other/astrid/docs/steward-notes/AI_BEINGS_AUTORESEARCH_JOB_ACTIONS_PLAN.md`
  Rationale: important design note, but separate from current code integration.
- `/Users/v/other/astrid/docs/steward-notes/ASTRID_CONTEXT_LIMITS_MODEL_SCALE_AND_MLX_HEADROOM_AUDIT.md`
  Rationale: runtime survey and capacity planning, not a direct code bundle.
- `/Users/v/other/astrid/docs/steward-notes/ASTRID_NOMENCLATURE_HARDENING_PROPOSAL.md`
  Rationale: naming and architecture cleanup proposal, best reviewed after the
  current behavior changes settle.

## Excluded Runtime Churn

Tracked `workspace/` changes and generated runtime artifacts were excluded from
the bundles on purpose. They should not drive code review decisions unless we
open a separate cleanup pass to untrack or relocate them.
