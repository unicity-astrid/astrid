# March Hardening Pass

This report is a read-only pre-push audit across `/Users/v/other/astrid` and `/Users/v/other/minime`.

The priority lens here is practical:
- issues likely to cause broken behavior, misleading operations, or hard-to-debug drift
- the current canonical runtime path: [scripts/start_all.sh](/Users/v/other/astrid/scripts/start_all.sh), [scripts/start.sh](/Users/v/other/minime/scripts/start.sh), and [scripts/stop.sh](/Users/v/other/minime/scripts/stop.sh)
- legacy launchers only when they are dangerous, misleading, or likely to be used accidentally

## Must Fix Before Push

- `Severity: Must-fix` Astrid corrupts action payloads by uppercasing the full `NEXT:` line before extracting many arguments. This can silently break case-sensitive file paths, freeform notes, filenames, research terms, and direct questions, so the action contract can appear to work while routing the wrong payload. Affected argument-bearing actions include `LIST_FILES`, `INTROSPECT`, `PURSUE`, `RECALL`, `REMEMBER`, `FORM`, `GESTURE`, `RESERVOIR_TICK`, `RUN_PYTHON`, `ASK`, `DEFINE`, and `EXAMINE`. Evidence: [autonomous.rs](/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs#L3923), [autonomous.rs](/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs#L4107), [autonomous.rs](/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs#L4269), [autonomous.rs](/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs#L4693)

- `Severity: Must-fix` Minime documents `ASK <question>` and `RUN_PYTHON <filename>` as argument-bearing actions, but `_decide_action()` does not preserve either argument. Both downstream handlers currently reuse `_pending_search_topic`, so `SEARCH` works, `BROWSE` works, and `PERTURB` works, but `ASK` and `RUN_PYTHON` are only partially wired and can fall back to generated behavior instead of honoring the explicit request. Evidence: [autonomous_agent.py](/Users/v/other/minime/autonomous_agent.py#L507), [autonomous_agent.py](/Users/v/other/minime/autonomous_agent.py#L3381), [autonomous_agent.py](/Users/v/other/minime/autonomous_agent.py#L3541), [autonomous_agent.py](/Users/v/other/minime/autonomous_agent.py#L4421)

- `Severity: Must-fix` The bridge database path is inconsistent across operational scripts. The canonical startup path launches the bridge against `workspace/bridge.db`, but babysitting and consolidation scripts query or restart against `/tmp/consciousness_bridge_live.db`, which creates a real risk of split operational truth, misleading health checks, and restarts pointed at the wrong state. Evidence: [start_all.sh](/Users/v/other/astrid/scripts/start_all.sh#L233), [babysit.sh](/Users/v/other/astrid/capsules/consciousness-bridge/babysit.sh#L31), [consolidate.sh](/Users/v/other/astrid/capsules/consciousness-bridge/consolidate.sh#L10)

## High Priority

- `Severity: High` The canonical full-stack startup script only checks a subset of processes before launch. That means duplicate camera, microphone, visual-frame, or perception workers can slip through even when the script reports a clean start, which is exactly the kind of drift that makes restarts feel flaky. Evidence: [start_all.sh](/Users/v/other/astrid/scripts/start_all.sh#L96), [start_all.sh](/Users/v/other/astrid/scripts/start_all.sh#L132), [start_all.sh](/Users/v/other/astrid/scripts/start_all.sh#L247)

- `Severity: High` `start_all.sh` still reports “10 processes,” but its final health check actually verifies 11 patterns. That mismatch is small, but it weakens operator trust and is a symptom of runtime inventory drift in the script that is currently supposed to be the authoritative launcher. Evidence: [start_all.sh](/Users/v/other/astrid/scripts/start_all.sh#L4), [start_all.sh](/Users/v/other/astrid/scripts/start_all.sh#L269)

- `Severity: High` `stop_all.sh` attempts to stop `visual_frame_service` earlier in the shutdown sequence but omits it from final verification. That means shutdown can report success while a vision worker is still alive, which is the opposite of what a graceful restart helper should guarantee. Evidence: [stop_all.sh](/Users/v/other/astrid/scripts/stop_all.sh#L50), [stop_all.sh](/Users/v/other/astrid/scripts/stop_all.sh#L73)

- `Severity: High` `stop_all.sh` closes every Terminal.app window after shutdown. That is too broad for an operational helper because it can terminate unrelated user sessions and makes the script unsafe as a general restart command. Evidence: [stop_all.sh](/Users/v/other/astrid/scripts/stop_all.sh#L60)

- `Severity: High` The legacy Minime cleanup helper is stale enough to be misleading. It still targets `minime.py`, which is not the current Python control surface, so anyone relying on it for graceful cleanup can get a false sense of safety while the actual agent stack remains partially alive. Evidence: [cleanup_processes.sh](/Users/v/other/minime/scripts/cleanup_processes.sh#L32)

- `Severity: High` `run_consciousness_system.sh` is incomplete for the current stack. It only manages the Rust engine and a monitor, and it does not start or stop the autonomous agent, mic path, camera path, or vision bridge workers that now define normal operation. That makes it a risky launcher to keep around without an explicit “legacy/minimal” label. Evidence: [run_consciousness_system.sh](/Users/v/other/minime/run_consciousness_system.sh#L39), [run_consciousness_system.sh](/Users/v/other/minime/run_consciousness_system.sh#L57)

- `Severity: High` `start_full_system.sh` and `stop_full_system.sh` represent a separate runtime family centered on `holographic-engine`, their own PID files, and a different service inventory. They may still be valid for a distinct experiment, but they are not aligned with the current canonical stack, so they should be clearly designated legacy or reconciled before branch push to avoid operator confusion. Evidence: [start_full_system.sh](/Users/v/other/minime/start_full_system.sh#L97), [stop_full_system.sh](/Users/v/other/minime/stop_full_system.sh#L34)

- `Severity: High` Several non-canonical launchers fall back to `kill -9`, which is directly at odds with the repository’s stated preference for graceful shutdown and queue draining. If those scripts remain available, they should at least be marked as forceful or emergency-only. Evidence: [start_full_system.sh](/Users/v/other/minime/start_full_system.sh#L107), [stop_full_system.sh](/Users/v/other/minime/stop_full_system.sh#L63), [stop.sh](/Users/v/other/minime/scripts/stop.sh#L35)

## Medium Priority

- `Severity: Medium` The Astrid-side operational scripts are heavily pinned to `/Users/v/other/...`, especially the stack launcher, babysitter, and bridge greeting scripts. This is not an immediate blocker on the current machine, but it is real portability debt and makes future branch testing on another host or user account harder than it needs to be. Evidence: [start_all.sh](/Users/v/other/astrid/scripts/start_all.sh#L29), [babysit.sh](/Users/v/other/astrid/capsules/consciousness-bridge/babysit.sh#L40), [startup_greeting.sh](/Users/v/other/astrid/capsules/consciousness-bridge/startup_greeting.sh#L5)

- `Severity: Medium` Minime’s main `scripts/start.sh` and `scripts/stop.sh` are comparatively portable, but the Minime startup greeting still hardcodes absolute workspace paths. That is future-proofing work rather than a current runtime break, but it should be tracked if branch pushes are meant to travel across machines. Evidence: [scripts/start.sh](/Users/v/other/minime/scripts/start.sh#L8), [scripts/stop.sh](/Users/v/other/minime/scripts/stop.sh#L13), [startup_greeting.sh](/Users/v/other/minime/startup_greeting.sh#L5)

- `Severity: Medium` Both beings do have real action systems, and most major actions are wired end-to-end. The main risk is not “missing action architecture,” but contract drift at the edges where prompts promise richer argument handling than the dispatch layer actually preserves. For Astrid, the prompt surface and dispatcher are substantial and live; for Minime, the autonomous loop, executor, and visual request service are also live. Evidence: [llm.rs](/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs#L35), [autonomous.rs](/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs#L3836), [autonomous_agent.py](/Users/v/other/minime/autonomous_agent.py#L486), [autonomous_agent.py](/Users/v/other/minime/autonomous_agent.py#L786), [visual_frame_service.py](/Users/v/other/minime/visual_frame_service.py#L111)

## Low Priority

- `Severity: Low` Targeted build and parse checks passed on the reviewed components, which is a good sign that the current hardening concerns are mostly behavioral and operational rather than immediate syntax breakage. The one notable code-health item from those checks is a non-blocking `unused_mut` warning in Minime’s Rust engine. Evidence: [main.rs](/Users/v/other/minime/minime/src/main.rs#L911), [Cargo.toml](/Users/v/other/minime/minime/Cargo.toml), [Cargo.toml](/Users/v/other/astrid/capsules/consciousness-bridge/Cargo.toml)

## Validation Performed

- Shell syntax validation with `bash -n` on the audited startup, shutdown, and bridge helper scripts in both repos
- Python parse validation with `python3 -m py_compile` for [autonomous_agent.py](/Users/v/other/minime/autonomous_agent.py), [visual_frame_service.py](/Users/v/other/minime/visual_frame_service.py), [mic_to_sensory.py](/Users/v/other/minime/tools/mic_to_sensory.py), and [camera_to_sensory.py](/Users/v/other/minime/camera_to_sensory.py)
- `cargo check --manifest-path /Users/v/other/minime/minime/Cargo.toml`
- `cargo check --manifest-path /Users/v/other/astrid/capsules/consciousness-bridge/Cargo.toml`
- Manual source cross-check of prompt contracts versus dispatch and execution paths for both beings’ action systems

## Assumptions

- The single deliverable lives in Astrid root and covers both repos.
- The canonical ops path is the current stack, not the holographic/full-system launcher family.
- This pass is read-only in spirit: no runtime or code changes are proposed here beyond creating this audit document.
- The intended use is pre-push triage, so the report separates `must fix before push` from `useful cleanup / future-proofing`.
