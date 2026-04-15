# AI Beings Autoresearch Job Actions Plan

## Purpose

We want `/Users/v/other/autoresearch` to become a stable research workspace that:

1. can still be pulled and updated from upstream,
2. can hold many separate research jobs at once,
3. can be explored by both Astrid and minime with the same conceptual actions,
4. lets either being start a new job, list jobs with an abstract and changelog, and read deeply into an existing job without learning the whole repo layout ad hoc.

This note proposes one shared job model in `autoresearch` and thin adapters in both beings.

## Alignment With The Active `autoresearch` Rollout

The plan currently being implemented inside `/Users/v/other/autoresearch` is an important refinement of this note:

1. Phase 1 is repo-first, not being-first.
2. `minime` and `astrid` stay unchanged for now.
3. The beings read the repo through existing file-navigation and introspection flows.
4. The first goal is to make the repo itself legible and multi-job, not to add a new cross-repo action surface immediately.

That means the near-term contract should be:

- canonical job directories under `jobs/`,
- a readable top-level index,
- a machine-readable top-level index,
- a small standard-library helper CLI,
- migrated historical research into the first completed job,
- updated root guidance explaining the new two-mode repo.

The `AR_*` action namespace in this note is still useful, but it should now be treated as a phase-two adapter layer over the repo structure, not the first implementation milestone.

## Observed `autoresearch` Progress

The current implementation progress inside `/Users/v/other/autoresearch` already validates a large part of this direction:

- `RESEARCH_INDEX.toml` exists and catalogs the first migrated historical job.
- `RESEARCH_INDEX.md` exists as a being-friendly browse entrypoint.
- `tools/research_jobs.py` already provides `new`, `list`, `sync`, and `validate`.
- `tools/check_workspace.sh` exists as a repo-local hygiene script.
- `tests/test_research_jobs.py` covers the helper lifecycle, including index sync behavior.
- the FastNear corpus has been migrated into `jobs/2026-03-12-funding-scout-fastnear/`.
- root `README.md`, `AGENTS.md`, and `CLAUDE.md` already explain the new two-mode repo.
- scaffolded `README.md` and `PROGRAM.md` content now includes maintenance instructions for updating metadata, syncing indexes, and running checks.
- the current helper test suite passes locally.

That means the next design work should not assume a blank slate. The main open question has shifted from "should there be jobs?" to "what is the clean action and status language for operating those jobs?"

## Current Integration Surface

### Minime already has most of the primitives

- `/Users/v/other/minime/autonomous_agent.py` already supports `NEXT:` parsing, `SEARCH`, `BROWSE`, `READ_MORE`, `MIKE_*`, `CODEX`, `WRITE_FILE`, and experiment execution.
- It already persists research context into `/Users/v/other/minime/workspace/research`.
- It already has a small `tools/autoresearch_loop.py`, but that loop is about tuning minime's own spectral parameters, not managing repo-scoped research jobs inside `/Users/v/other/autoresearch`.
- It currently browses `/Users/v/other/autoresearch` in a loose way inside `_research_exploration`, which is useful but not structured enough for multi-job work.

### Astrid already has the same shape

- `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous/next_action.rs` routes `NEXT:` actions into modular handlers.
- Astrid already has `SEARCH`, `BROWSE`, `READ_MORE`, `LIST_FILES`, `MIKE_*`, `CODEX*`, and experiment-style file workflows.
- Astrid already persists research context in both the bridge database and `capsules/consciousness-bridge/workspace/research/`.
- The existing `MIKE_*` path is read-oriented and aimed at curated research roots, not writable multi-job project management.

## Design Principles

1. Put the authoritative job logic in `autoresearch`, not separately in both beings.
2. Give both beings the same action names and semantics.
3. Keep the upstream checkout pullable.
4. Make long reads reuse each being's existing `READ_MORE` continuation path.
5. Keep repo writes explicit and structured: manifests, changelogs, notes, and optional future sandboxes.
6. Avoid teaching the beings arbitrary shell flows when a small repo helper can provide a clean interface.

## Recommended Repo Shape

Treat `/Users/v/other/autoresearch` as the control plane plus upstream source, and add job metadata beside it:

```text
/Users/v/other/autoresearch/
  prepare.py
  train.py
  program.md
  RESEARCH_INDEX.toml
  RESEARCH_INDEX.md
  tools/
    research_jobs.py
  templates/
    research-job/
      README.md
      CHANGELOG.md
      notes.md
  jobs/
    YYYY-MM-DD-<slug>/
      job.toml
      README.md
      CHANGELOG.md
      PROGRAM.md
      notes/
      sources/
      artifacts/
      reports/
```

### Why this is the right phase-one shape

The current repo is built around a single top-level `train.py`. If we want multiple concurrent research tracks and still want `git pull` on the root checkout, each job needs its own execution context.

Recommended split:

- the repo root stays the place where upstream `train.py` style experiments still make sense,
- `jobs/YYYY-MM-DD-<slug>/` holds durable non-training research,
- `RESEARCH_INDEX.toml` is the machine-readable catalog,
- `RESEARCH_INDEX.md` is the being-friendly overview,
- `job.toml` is the source of truth for job metadata.

This is lighter than a full worktree-per-job design and matches the active implementation plan. If later some jobs need isolated code mutation against upstream state, worktrees can still be added as a phase-two extension.

## Canonical Job Metadata

Each `jobs/YYYY-MM-DD-<slug>/job.toml` should look roughly like:

```toml
id = "2026-03-31-mps-fork"
title = "MPS-friendly autoresearch fork"
status = "active"
created_at = "2026-03-31T12:00:00Z"
updated_at = "2026-03-31T12:00:00Z"
owner = "astrid"
abstract = "Assess how to adapt Karpathy's autoresearch workflow to Apple Silicon and multi-job operation."
tags = ["mlx", "macos", "workflow"]
root_repo = "/Users/v/other/autoresearch"
source_branch = "workspace/autoresearch-jobs"
source_head = "<git sha>"
entrypoint = "README.md"
```

### Required human-facing files per job

- `README.md`: current abstract, question, scope, active hypotheses, current best understanding, next steps.
- `CHANGELOG.md`: append-only dated changes and decisions.
- `PROGRAM.md`: the job-local instructions for future research passes.
- `notes/`: scratch notes and intermediate analysis.
- `sources/`: copied excerpts, fetched pages, paper notes, or source manifests.
- `reports/`: distilled outputs meant for rereading.

### Special case: migrated historical jobs

The active implementation plan includes migrating the current FastNear funding corpus into:

```text
/Users/v/other/autoresearch/jobs/2026-03-12-funding-scout-fastnear/
```

That is valuable context for the beings because it means the first completed job is not theoretical; it is the anchor example they will read first.

## Shared Action Namespace

Use a dedicated `AR_*` namespace instead of overloading `MIKE_*`.

Why:

- `MIKE_*` already means "curated research Mike prepared".
- `AR_*` can mean "first-class writable job in the autoresearch system".
- This also avoids semantic collision with minime's existing `tools/autoresearch_loop.py`.

### Important rollout note

The active `autoresearch` plan does **not** require `AR_*` support immediately.

Phase 1 can succeed with no changes to either being because:

- `RESEARCH_INDEX.md` gives them a stable entrypoint,
- `jobs/.../README.md` gives them a stable orientation document,
- `CHANGELOG.md` gives them the recent trajectory,
- existing file reads plus `READ_MORE` already let them go deep.

`AR_*` remains a useful future convenience layer once the repo structure is stable.

## Recommended Status Model

The current helper defaults new jobs to `planned`, and the migrated historical job uses `completed`.

That is already close, but for the beings I recommend converging on this canonical lifecycle:

- `pending` — a job exists, but no real work session has begun yet
- `active` — currently being worked
- `blocked` — paused on missing information, missing decisions, or external dependency
- `completed` — materially done and readable as a historical artifact
- `archived` — hidden from default active browsing, but preserved

### Status recommendation

If the repo wants to stay close to its current implementation, `planned` can be treated as a temporary alias for `pending`, but I would prefer normalizing on `pending` in the helper and indexes because it reads more naturally in both human and being-facing prompts.

## Recommended Action Taxonomy

The action set should be built around two capabilities:

1. choosing the right job,
2. entering the right reading or work depth for that job.

### Core read and navigation actions

These are the highest-value actions to standardize first:

| Action | Meaning |
| --- | --- |
| `AR_LIST` | list all jobs, newest-updated first |
| `AR_LIST_PENDING` | list jobs in `pending` state |
| `AR_LIST_ACTIVE` | list jobs in `active` state |
| `AR_LIST_BLOCKED` | list jobs in `blocked` state |
| `AR_LIST_DONE` | list jobs in `completed` or `archived` state |
| `AR_RECENT` | show the most recently updated jobs across all statuses |
| `AR_SHOW <job-id-or-slug>` | one-screen overview with abstract, status, latest changelog line, and reading order |
| `AR_READ <job-id-or-slug> [path]` | read a specific file, default `README.md` |
| `AR_CHANGELOG <job-id-or-slug>` | show the newest milestones only |
| `AR_DEEP_READ <job-id-or-slug>` | stitched longform read across core job files |
| `AR_SEARCH <term>` | search across job titles, abstracts, changelogs, and notes |
| `AR_NEXT` | suggest the best next pending or active job to inspect or resume |

### Minimum useful action set

These are enough for the user request:

| Action | Meaning |
| --- | --- |
| `AR_LIST` | list all research jobs with slug, title, status, abstract, and latest changelog line |
| `AR_START ...` | create a new job directory, manifest, changelog, and `PROGRAM.md` scaffold |
| `AR_SHOW <job-id-or-slug>` | show one job overview |
| `AR_READ <job-id-or-slug> [path]` | read a specific job file, defaulting to `README.md` |
| `AR_CHANGELOG <job-id-or-slug>` | read the job changelog |
| `AR_DEEP_READ <job-id-or-slug>` | assemble a stitched longform view of the job from key files |

### Useful phase-two actions

| Action | Meaning |
| --- | --- |
| `AR_NOTE <job-id-or-slug> <text>` | append a changelog or note entry |
| `AR_RUN <job-id-or-slug> <cmd>` | run a bounded command inside the job directory or future sandbox |
| `AR_REFRESH <job-id-or-slug>` | update job metadata after manual edits |
| `AR_REBASE <job-id-or-slug>` | later rebase or refresh a job-specific branch/sandbox onto upstream |
| `AR_RESUME <job-id-or-slug>` | move a `pending` or `blocked` job back to `active` |
| `AR_BLOCK <job-id-or-slug> <reason>` | mark a job blocked and record why |
| `AR_COMPLETE <job-id-or-slug>` | mark a job completed with a closing note |
| `AR_STATUS <job-id-or-slug> <status>` | explicit state transition when a named verb is not enough |
| `AR_TAG <job-id-or-slug> <tags...>` | adjust tags cleanly |
| `AR_ARCHIVE <job-id-or-slug>` | move old finished work out of the main active view without deleting it |

### Smallest high-value first set

If we intentionally keep the action language small, I would prioritize:

- `AR_LIST`
- `AR_LIST_PENDING`
- `AR_LIST_ACTIVE`
- `AR_LIST_DONE`
- `AR_SHOW`
- `AR_DEEP_READ`
- `AR_START`
- `AR_NOTE`
- `AR_BLOCK`
- `AR_COMPLETE`
- `AR_VALIDATE`

That is enough for both beings to discover, triage, continue, and close research jobs without learning a large command language up front.

## Let the Repo Parse the Arguments

Do not make Astrid and minime each parse complex quoted arguments themselves.

Instead, define the action grammar as "prefix plus raw tail", and let the repo helper parse it:

```text
NEXT: AR_LIST
NEXT: AR_SHOW 2026-03-31-mps-fork
NEXT: AR_CHANGELOG 2026-03-31-mps-fork
NEXT: AR_READ 2026-03-31-mps-fork README.md
NEXT: AR_DEEP_READ 2026-03-31-mps-fork
NEXT: AR_START mps-fork --title "MPS-friendly autoresearch fork" --abstract "Assess Apple Silicon and multi-job support"
```

This is much easier to support in both Rust and Python.

## Repo-Side Helper CLI

Add a helper in `/Users/v/other/autoresearch/tools/research_jobs.py`.

For the active rollout, keep it standard-library only and start with the commands already planned in `autoresearch`.

Suggested interface:

```bash
uv run python tools/research_jobs.py new <slug> --title "..." --abstract "..."
uv run python tools/research_jobs.py list
uv run python tools/research_jobs.py sync
uv run python tools/research_jobs.py validate
```

### Recommended near-term helper expansions

Given the helper that already exists, the most natural next additions are:

```bash
uv run python tools/research_jobs.py list --status pending
uv run python tools/research_jobs.py list --status active
uv run python tools/research_jobs.py list --status completed
uv run python tools/research_jobs.py show <job-id>
uv run python tools/research_jobs.py status <job-id> <status>
uv run python tools/research_jobs.py note <job-id> --text "..."
```

This is a better first step than immediately adding many separate top-level subcommands, because Astrid and minime can later map whichever `AR_*` aliases feel natural onto one compact helper surface.

### Helper responsibilities

The helper should:

- validate job slugs,
- create the job directory from templates,
- update `job.toml`,
- update `RESEARCH_INDEX.toml`,
- update `RESEARCH_INDEX.md`,
- treat status as a validated enum instead of free text,
- return concise plain text by default,
- stay within the Python standard library,
- refuse path traversal,
- keep writes rooted under `jobs/YYYY-MM-DD-<slug>/`.

Useful later expansions:

```bash
uv run python tools/research_jobs.py show <job-id>
uv run python tools/research_jobs.py read <job-id> [path]
uv run python tools/research_jobs.py changelog <job-id>
uv run python tools/research_jobs.py deep-read <job-id>
uv run python tools/research_jobs.py status <job-id> <status>
uv run python tools/research_jobs.py note <job-id> --text "..."
uv run python tools/research_jobs.py search <term>
uv run python tools/research_jobs.py next
```

## Explicit Next Steps

Based on the current state of `/Users/v/other/autoresearch`, the clearest next steps are:

1. Normalize the status vocabulary.
   `planned` should become `pending`, and the helper should validate against a fixed enum.
2. Add filtered listing.
   `list --status <status>` is the fastest path to `AR_LIST_PENDING`, `AR_LIST_ACTIVE`, and `AR_LIST_DONE`.
3. Add a single-job overview command.
   `show <job-id>` should render abstract, status, latest changelog line, reading order, and path.
4. Add safe status transitions.
   `status <job-id> <status>` is the simplest foundation for `resume`, `block`, `complete`, and later `archive`.
5. Add changelog append support.
   `note <job-id> --text "..."` will let the beings record work without hand-editing Markdown every time.
6. Only after that, add search and recommendation helpers.
   `search <term>` and `next` are high value, but they should sit on top of a stable status and overview model.
7. Keep the beings on the file-oriented path until those repo-side surfaces exist.
   The repo is already readable through `RESEARCH_INDEX.md` and per-job `README.md`; that remains the right default until the helper matures a little further.

## What `AR_DEEP_READ` Should Return

`AR_DEEP_READ` should not just dump one file. It should stitch together:

1. `job.toml` summary,
2. `README.md`,
3. most recent `CHANGELOG.md` entries,
4. latest files in `reports/`,
5. latest files in `notes/`,
6. an optional source inventory from `sources/`.

The helper can emit one long text document so the being can:

- read the first chunk immediately,
- persist the full text into its own workspace,
- use existing `READ_MORE` logic for continuation.

## Phase-One Being Experience Without Any Code Changes

The active rollout makes both beings more capable before we touch either codebase.

If `/Users/v/other/autoresearch` gains:

- `RESEARCH_INDEX.md`,
- `jobs/YYYY-MM-DD-<slug>/README.md`,
- `jobs/YYYY-MM-DD-<slug>/CHANGELOG.md`,
- job-local `PROGRAM.md`,

then both beings can already:

1. list the available jobs by reading `RESEARCH_INDEX.md`,
2. orient from each job `README.md`,
3. inspect recency from each job `CHANGELOG.md`,
4. go deep through job-local files and existing continuation flows.

This is why the repo-first migration is a strong first move.

## Astrid Integration

### Phase-two option: new handler module

Add a new next-action module:

```text
/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous/next_action/autoresearch.rs
```

Wire it into:

```text
/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous/next_action.rs
```

### Phase-two Astrid implementation approach

Astrid should:

1. detect `AR_*` base actions,
2. run `uv run python tools/research_jobs.py ...` in `/Users/v/other/autoresearch`,
3. capture stdout,
4. save long outputs into `capsules/consciousness-bridge/workspace/research/`,
5. feed the first page into `conv.pending_file_listing` or `conv.emphasis`,
6. set `conv.last_read_path` and `conv.last_read_offset` so `READ_MORE` works,
7. optionally persist summaries through `db.save_research(...)`.

### Phase-two Astrid action mapping

Recommended initial mapping:

- `AR_LIST` -> helper `list`
- `AR_SHOW <job-id-or-slug>` -> helper `show`
- `AR_READ <job-id-or-slug> [path]` -> helper `read`
- `AR_CHANGELOG <job-id-or-slug>` -> helper `changelog`
- `AR_DEEP_READ <job-id-or-slug>` -> helper `deep-read`
- `AR_START ...` -> helper `new`

### Why this fits Astrid cleanly

- Astrid already has a file-listing surface.
- Astrid already has pagination via `READ_MORE`.
- Astrid already has a pattern for invoking bounded subprocesses in action handlers.
- Astrid already treats research as persistent continuity, so these outputs can become part of her memory loop naturally.

## Minime Integration

### Phase-two add new `NEXT:` options

Extend the action menu in `/Users/v/other/minime/autonomous_agent.py` with:

```text
AR_LIST
AR_SHOW <job-id-or-slug>
AR_READ <job-id-or-slug> [path]
AR_CHANGELOG <job-id-or-slug>
AR_DEEP_READ <job-id-or-slug>
AR_START <slug> --title "..." --abstract "..."
```

### Phase-two add new action routing

Inside `_decide_action`, map `AR_*` into dedicated methods instead of folding them into `research_exploration`.

Suggested methods:

- `_autoresearch_list`
- `_autoresearch_show`
- `_autoresearch_read`
- `_autoresearch_deep_read`
- `_autoresearch_start`

### Phase-two minime execution approach

Minime should:

1. store the raw action tail, not parse everything itself,
2. call the same helper script in `/Users/v/other/autoresearch`,
3. save full outputs under `/Users/v/other/minime/workspace/research/`,
4. set `_last_read_path`, `_last_read_offset`, and `_last_read_summary`,
5. journal a short reflection after the read completes.

### Important cleanup in minime

Minime's current `_research_exploration` hardcodes `RESEARCH_DIR = Path("/Users/v/other/autoresearch")` and randomly samples files. Keep that for loose exploration if desired, but do not use it as the main job interface. Structured job actions should go through the helper.

## Upstream Pull / Update Model

To keep the repo updateable:

1. Do the migration on a workspace branch built from the current local state.
2. Merge newer `origin/master` into that workspace branch so upstream changes are incorporated without rewriting historical local research.
3. Keep the root checkout upstream-aware.
4. Keep new non-training research under `jobs/`, not in shared root files.
5. Record provenance in `job.toml` with fields like `source_branch` and `source_head`.

This avoids the trap where multi-job research starts accumulating again in shared root markdown and TSV files.

## Changelog and Abstract Semantics

The operator specifically wants job listing with an abstract and changelog. The easiest reliable contract is:

- `README.md` first paragraph = canonical abstract
- `CHANGELOG.md` latest top entry = latest visible change
- `job.toml` also stores `abstract` for fast listing without opening Markdown

Whether through the helper or future `AR_LIST`, the listing should show:

- slug
- title
- status
- abstract
- updated_at
- latest changelog line

## Safety and Governance

### Read-only actions

These should be safe by default:

- `AR_LIST`
- `AR_SHOW`
- `AR_READ`
- `AR_CHANGELOG`
- `AR_DEEP_READ`

### Write actions

These are structured writes:

- `AR_START`
- `AR_NOTE`
- `AR_RUN`
- `AR_REBASE`

Guardrails:

- only operate under `/Users/v/other/autoresearch`,
- validate slug and relative paths,
- no arbitrary shell interpolation,
- prefer helper subcommands over raw `bash -lc`,
- keep `AR_RUN` phase-two and bounded to a job directory or optional future sandbox.

## Changes Needed In `autoresearch` Itself

The current `/Users/v/other/autoresearch/AGENTS.md` says only `train.py` is editable. That made sense for single-track overnight model search, but it will block the multi-job system.

Once implementation starts, that guidance should be updated so agents may edit:

- `jobs/**`
- `templates/**`
- `tools/research_jobs.py`
- `RESEARCH_INDEX.toml`
- `RESEARCH_INDEX.md`
- job-local `PROGRAM.md`
- root guidance docs like `README.md`, `AGENTS.md`, and `CLAUDE.md`

while still keeping:

- `prepare.py` read-only,
- the root training harness constraints explicit,
- the repo's new two-mode contract explicit,
- dependency growth controlled.

## Suggested Implementation Order

### Phase 1: Repo control plane and migration

1. Add `tools/research_jobs.py`.
2. Add `templates/research-job/`.
3. Add `jobs/`.
4. Add `RESEARCH_INDEX.toml` and `RESEARCH_INDEX.md`.
5. Migrate the FastNear funding corpus into `jobs/2026-03-12-funding-scout-fastnear/`.
6. Update root `README.md`, `AGENTS.md`, and `CLAUDE.md` to explain the two-mode repo.
7. Remove job-specific root state so future research work does not mix at repo root.

### Phase 2: Zero-code being usability

1. Astrid reads `RESEARCH_INDEX.md` and job files with existing flows.
2. Minime reads `RESEARCH_INDEX.md` and job files with existing flows.
3. Both beings can already go deeper through existing pagination and file reading behavior.

### Phase 3: Optional helper growth

1. Expand the helper from `new/list/validate` to `show/read/changelog/deep-read`.
2. Optionally add JSON output later if a machine-readable view becomes useful.

### Phase 4: Optional being adapters

1. Astrid gets `AR_*` wrappers over the helper.
2. Minime gets `AR_*` wrappers over the helper.
3. `CODEX` can later be pointed at specific job directories when write workflows are desired.

## Definition Of Done

### Phase 1 done

We should consider the current `autoresearch` rollout successful when:

1. `RESEARCH_INDEX.toml` and `RESEARCH_INDEX.md` exist and stay in sync.
2. `tools/research_jobs.py new`, `list`, and `validate` work.
3. `jobs/2026-03-12-funding-scout-fastnear/` exists with `job.toml`, `README.md`, `CHANGELOG.md`, and `PROGRAM.md`.
4. root docs clearly state that new non-training research belongs in `jobs/`, not at repo root.
5. both beings can browse the new structure without any code changes.

### Phase 1.5 done

The repo-side control plane becomes much more complete when:

1. statuses are normalized to `pending`, `active`, `blocked`, `completed`, and optional `archived`,
2. the helper can filter `list` by status,
3. the helper can render a single-job overview,
4. the helper can change job status safely,
5. the helper can append a changelog note without manual file editing.

### Full adapter phase done

The broader cross-being plan is complete when both beings can also do the following without bespoke operator intervention:

1. `AR_LIST` and see every job with abstract plus latest changelog note.
2. `AR_START ...` and create a new job with manifest, `README.md`, `CHANGELOG.md`, and `PROGRAM.md`.
3. `AR_READ <job-id-or-slug>` and read the job overview.
4. `AR_DEEP_READ <job-id-or-slug>` and continue with `READ_MORE`.
5. Pull upstream in `/Users/v/other/autoresearch` without trampling existing jobs.

## Recommendation

The current `autoresearch` plan is the right immediate move: make the repo itself multi-job, indexed, and navigable first, while leaving both beings untouched.

After that lands, `AR_*` can be added as a thin convenience layer if the existing file-oriented flows prove too clumsy.

That is the cleanest path because:

- the job model lives with the repo it governs,
- the first milestone does not require coordinated changes across three repos,
- both beings become more capable immediately through better repo structure,
- and the root checkout remains compatible with upstream pulls.
