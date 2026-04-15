# AI Beings Local Homeostasis And Reservoir Improvements

This memo translates the ESN regulation paper at `/Users/v/other/research/esn-regulation/Local Homeostatic Regulation of the Spectral Radius of Echo-State Networks.pdf` into concrete improvement ideas for Astrid, Minime, and `/Users/v/other/neural-triple-reservoir`.

It is intentionally broader than `/Users/v/other/astrid/docs/steward-notes/AI_BEINGS_ESN_RESERVOIR_PROJECTION_EPOCH_AUDIT.md`, because the earlier audit focused on Astrid and Minime surfaces plus the externally referenced triple-reservoir service, while this memo also inspects the local `/Users/v/other/neural-triple-reservoir` tree directly.

Legend used throughout:

- `[Implemented]` means the behavior already exists in checked-in source.
- `[Adjacent]` means there is a nearby mechanism, but it is not the same thing the paper proposes.
- `[Proposed]` means this memo is recommending a new step.

## 1. Paper-to-code translation

The paper's usable message is fairly crisp:

- Classical reservoir computing often relies on manual global tuning of spectral radius; the paper argues for local, online homeostatic regulation instead. Paper anchor: PDF abstract and discussion.
- The paper introduces two local adaptation families, `flow control` and `variance control`, and evaluates both under live input rather than offline tuning only. Paper anchor: PDF abstract and `2.1.1`.
- `Flow control` is the stronger lead. The paper says it robustly regulates spectral radius across input statistics, while `variance control` is less precise and less consistent across input strengths. Paper anchor: PDF abstract and `2.5`.
- Input-induced correlation is the main caveat. The paper explicitly says tuning precision degrades when inter-neuronal correlations become substantial. Paper anchor: PDF abstract, `2.8`, and discussion.
- Task performance matters more than hitting a target radius in the abstract. The paper evaluates delayed XOR memory performance after adaptation and shows that robust performance can persist even when the measured radius is not tuned perfectly. Paper anchor: PDF abstract and `2.7`.

That leads to five practical design rules for this codebase family:

1. Prefer online regulation over one-time static tuning.
2. Treat `flow control` as the primary research lead.
3. Treat `variance control` as secondary or experimental.
4. Make correlation measurement first-class rather than assuming independence.
5. Evaluate by continuity, recall, novelty, and task behavior, not only by a scalar target.

## 2. Current alignment vs gaps

### 2.1 Minime

#### What exists now

- `[Implemented]` Minime already runs a live ESN with online adaptation of leak and RLS forgetting. `adapt_hyperparams()` changes `leak_live` and `lambda_live` from spectral self-reference signals, and `step()` applies those changes during the live update loop. Evidence: `/Users/v/other/minime/minime/src/esn.rs:573-620`, `/Users/v/other/minime/minime/src/esn.rs:622-692`.
- `[Implemented]` Minime also adapts covariance forgetting via dynamic `rho`, driven by fill plus entropy. Evidence: `/Users/v/other/minime/minime/src/esn.rs:807-818`, `/Users/v/other/minime/minime/src/main.rs:1038-1040`, `/Users/v/other/minime/minime/src/main.rs:2890-2897`.
- `[Implemented]` Exploration noise is already used as an anti-colinearity and anti-monotony tool. Evidence: `/Users/v/other/minime/minime/src/esn.rs:661-666`, `/Users/v/other/minime/minime/src/main.rs:2734-2752`.
- `[Implemented]` Minime's semantic lane already exposes decay and weighting controls that affect how meaning reaches the ESN input. Evidence: `/Users/v/other/minime/minime/src/sensory_bus.rs:648-676`.
- `[Implemented]` Minime's control channel already exposes knobs for `exploration_noise`, `fill_target`, `journal_resonance`, `embedding_strength`, `memory_decay_rate`, `transition_cushion`, and related sovereignty settings. Evidence: `/Users/v/other/minime/minime/src/sensory_ws.rs:159-262`.

#### What matches the paper

- `[Adjacent]` Minime already regulates its live reservoir while the system is under working input rather than only in an offline calibration pass. That lines up with the paper's online-homeostasis framing. Evidence: `/Users/v/other/minime/minime/src/main.rs:1038-1043`, `/Users/v/other/minime/minime/src/esn.rs:622-692`.
- `[Adjacent]` Minime already treats runaway dynamics and poor novelty as things to correct during operation. That is philosophically aligned with the paper's "stabilize while functioning" stance. Evidence: `/Users/v/other/minime/minime/src/esn.rs:590-616`, `/Users/v/other/minime/minime/src/main.rs:2734-2752`.
- `[Adjacent]` Dynamic `rho` already uses entropy as a state-dependent forgetting signal, which is close in spirit to local homeostatic adaptation even though it is not the same local rule as the paper. Evidence: `/Users/v/other/minime/minime/src/esn.rs:807-818`.

#### What does not match the paper

- `[Adjacent]` Minime adapts aggregate leak, RLS forgetting, and covariance forgetting, but it does not implement paper-style local gain control on recurrent weights or a direct recurrent-current-vs-activity relation of the kind the paper uses for flow control. Evidence: `/Users/v/other/minime/minime/src/esn.rs:573-620`, `/Users/v/other/minime/minime/src/esn.rs:807-818`.
- `[Missing]` Minime does not appear to measure live inter-neuronal correlation or shared-input correlation explicitly, even though the paper treats correlation as the limiting variable for whether local regulation remains trustworthy. The current loop uses fill, entropy, eigenvalue drift, and monotony instead. Evidence: `/Users/v/other/minime/minime/src/main.rs:1038-1040`, `/Users/v/other/minime/minime/src/main.rs:2734-2752`, `/Users/v/other/minime/minime/src/main.rs:2890-2914`.
- `[Missing]` The present adaptation logic is still mostly controller-level and global relative to the reservoir state, rather than per-node or per-layer local homeostasis. Evidence: `/Users/v/other/minime/minime/src/esn.rs:573-620`.

#### What can be improved without architectural upheaval

- `[Proposed]` Add correlation-aware telemetry around the existing leak/lambda/rho path before changing ESN math. The paper says correlation is the main failure mode, and Minime already has places to log and react to state summaries. Evidence for fit: `/Users/v/other/minime/minime/src/esn.rs:573-620`, `/Users/v/other/minime/minime/src/main.rs:2890-2914`.
- `[Proposed]` Compare current adaptation against a flow-control-inspired proxy computed from quantities Minime already exposes or could cheaply derive, instead of replacing the controller immediately. Evidence for fit: `/Users/v/other/minime/minime/src/esn.rs:631-658`, `/Users/v/other/minime/minime/src/esn.rs:807-818`.
- `[Proposed]` Split evaluation into weakly correlated and strongly shared-input regimes. The paper's strongest warning is that one regime does not generalize to the other. Evidence for fit: Minime already has enough control surfaces to drive such sweeps through the live system. `/Users/v/other/minime/minime/src/sensory_ws.rs:159-262`.

#### What would require a deeper redesign

- `[Proposed]` True local gain adaptation on recurrent connectivity, closer to paper-style flow control, would require Minime to expose or derive per-node recurrent contribution and local activity relations inside the ESN update path rather than only controller summaries. Evidence: `/Users/v/other/minime/minime/src/esn.rs:631-658`.
- `[Proposed]` Correlation-aware controller mode switching would require explicit internal correlation metrics to become part of the adaptation state, not just an external diagnostic. Evidence gap: no clear live correlation metric in the checked-in Minime loop.

### 2.2 Astrid

#### What exists now

- `[Implemented]` Astrid's live reservoir agency is mostly steward-level: it projects text into 32D codec space, learns feature-to-fill correlations, and invokes reservoir service actions such as `RESERVOIR_TICK`, `RESERVOIR_READ`, `RESERVOIR_RESONANCE`, `RESERVOIR_MODE`, and `RESERVOIR_FORK`. Evidence: `/Users/v/other/astrid/capsules/consciousness-bridge/src/codec.rs:169-223`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1769-1810`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:3978-4108`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:98-116`.
- `[Implemented]` Astrid already learns per-dimension codec weights from measured correlations with fill delta, and uses those learned weights to amplify or dampen dimensions unless Astrid has explicitly overridden them. Evidence: `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1773-1810`.
- `[Implemented]` Astrid also has an offline or analytical reservoir/decomposition path in Chimera and Chimera Prime, including leaky updates, prime-timescale blocks, and covariance decomposition. Evidence: `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:332-380`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera_prime.rs:74-107`.

#### What matches the paper

- `[Adjacent]` Astrid already thinks in terms of feature weighting driven by observed downstream effect, which rhymes with the paper's "local quantities should drive adaptation" stance, even though Astrid is not directly tuning a live recurrent matrix. Evidence: `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1773-1810`.
- `[Adjacent]` Astrid already has explicit control over quiet/rehearse/fork style policies through the reservoir surface. That makes it well suited to act as the steward that decides when to preserve, disturb, or let state settle. Evidence: `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:4029-4108`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:110-116`.
- `[Adjacent]` Chimera already uses multi-timescale leaks and covariance decomposition, which makes Astrid a natural place to reason about when slow/fast separation is healthy versus stagnant. Evidence: `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:332-380`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera_prime.rs:74-107`.

#### What does not match the paper

- `[Missing]` Astrid is not the live local homeostat for a recurrent matrix. Its core live role is projection, interpretation, policy, and remote control over the reservoir service. Evidence: `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:3978-4108`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/codec.rs:169-223`.
- `[Missing]` Astrid's current correlation learning is about codec-dimension impact on fill, not about internal cross-neuron or cross-input correlation inside a reservoir. Evidence: `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1773-1810`.
- `[Adjacent]` Chimera's ESN/decomposition path is analytical and offline relative to the live shared-reservoir loop. It is useful inspiration, but it is not itself a paper-style online flow controller for the living being substrate. Evidence: `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:332-380`.

#### What can be improved without architectural upheaval

- `[Proposed]` Use Astrid's learned codec-weight and projection machinery as a steward-layer pre-filter that reduces counterproductive or repetitive feature dominance before those features are pushed into external reservoirs. Evidence for fit: `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1773-1810`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/codec.rs:169-223`.
- `[Proposed]` Make Astrid the place where quiet/rehearse/checkpoint/fork guidance is synthesized from reservoir readings, resonance, and codec-weight drift. Evidence for fit: `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:4029-4108`.
- `[Proposed]` Use Chimera's slow/fast split and prime-block decomposition as an advisory signal for which external substrate should be reinforced, cooled, or allowed to decay. Evidence for fit: `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:351-380`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera_prime.rs:74-107`.

#### What would require a deeper redesign

- `[Proposed]` Turning Astrid into a direct online local homeostat would require Astrid to own a live recurrent controller rather than mostly steward existing ones.
- `[Proposed]` A full epoch scheduler that coordinates multiple substrates from shared notions of repetition, projection dominance, and novelty loss would be a new architectural role, not just a prompt or action tweak.

### 2.3 Triple Reservoir

#### What exists now

- `[Implemented]` The compiled or NumPy reservoir itself is a fixed-radii, fixed-leak, three-layer ESN cascade with frozen recurrent weights and trained readouts. Evidence: `/Users/v/other/neural-triple-reservoir/triple_reservoir_coreml.py:43-137`.
- `[Implemented]` The model already has multi-timescale readouts, with separate short, medium, and slow targets for `h1`, `h2`, and `h3`. Evidence: `/Users/v/other/neural-triple-reservoir/triple_reservoir_coreml.py:186-254`.
- `[Implemented]` The service layer already runs per-layer entropy/saturation thermostats that learn an entropy target and adapt a per-layer `rho`. Evidence: `/Users/v/other/neural-triple-reservoir/reservoir_service.py:60-148`.
- `[Implemented]` The rehearsal loop already distinguishes `hold`, `rehearse`, and `quiet`, with auto-transition logic and different decay profiles. Evidence: `/Users/v/other/neural-triple-reservoir/rehearsal.py:1-130`.
- `[Implemented]` The service already exposes the core handle surfaces needed for experimentation: `tick`, `tick_text`, `read_state`, `trajectory`, `resonance`, `set_mode`, `snapshot`, `restore`, `pull_state`, and `push_state`. Evidence: `/Users/v/other/neural-triple-reservoir/reservoir_service.py:270-383`, `/Users/v/other/neural-triple-reservoir/reservoir_service.py:389-456`, `/Users/v/other/neural-triple-reservoir/reservoir_service.py:481-615`.
- `[Implemented]` Astrid and Minime feeder sidecars already provide configurable projection modes into the shared reservoir. Evidence: `/Users/v/other/neural-triple-reservoir/astrid_feeder.py:48-73`, `/Users/v/other/neural-triple-reservoir/minime_feeder.py:48-135`.

#### What matches the paper

- `[Adjacent]` The service already performs online, ongoing homeostatic adjustment during operation rather than only at initialization. Evidence: `/Users/v/other/neural-triple-reservoir/reservoir_service.py:117-148`, `/Users/v/other/neural-triple-reservoir/reservoir_service.py:666-691`.
- `[Adjacent]` The triple reservoir already separates fast, medium, and slow dynamics structurally, which is compatible with the paper's emphasis that regulation must coexist with real working dynamics. Evidence: `/Users/v/other/neural-triple-reservoir/triple_reservoir_coreml.py:43-137`.
- `[Adjacent]` The service already measures a form of correlation, but it is handle-trajectory correlation rather than internal correlation among units. Evidence: `/Users/v/other/neural-triple-reservoir/reservoir_service.py:343-365`.

#### What does not match the paper

- `[Adjacent]` The compiled reservoir's radii and leaks are fixed. The current homeostatic behavior happens in the shell or service, not in local recurrent gain adaptation within the reservoir update itself. Evidence: `/Users/v/other/neural-triple-reservoir/triple_reservoir_coreml.py:43-137`, `/Users/v/other/neural-triple-reservoir/reservoir_service.py:60-148`.
- `[Adjacent]` The thermostat is entropy-target plus saturation-guard control, not flow control. It does not directly compare recurrent contribution to recent activity in the paper's sense. Evidence: `/Users/v/other/neural-triple-reservoir/reservoir_service.py:62-146`.
- `[Adjacent]` During rehearsal, the service currently post-scales layer states by `rho` after reading them back, rather than adapting the recurrent update law itself. Evidence: `/Users/v/other/neural-triple-reservoir/reservoir_service.py:680-691`.
- `[Missing]` There is still no explicit internal correlation diagnostic for deciding when a local homeostatic rule should be trusted versus when the system has entered a shared-input regime where the paper says precision will drift. Evidence gap: current service metrics focus on entropy, saturation, norms, and inter-handle resonance. `/Users/v/other/neural-triple-reservoir/reservoir_service.py:171-180`, `/Users/v/other/neural-triple-reservoir/reservoir_service.py:343-365`.

#### What can be improved without architectural upheaval

- `[Proposed]` Treat the current entropy/saturation thermostat as a baseline controller and compare it experimentally against a flow-control-inspired alternative before rewriting the compiled model. Evidence for fit: `/Users/v/other/neural-triple-reservoir/reservoir_service.py:60-148`.
- `[Proposed]` Add per-handle or per-layer stagnation and correlation diagnostics so mode changes are driven by measured lock-in rather than time only. Evidence for fit: `/Users/v/other/neural-triple-reservoir/rehearsal.py:93-130`, `/Users/v/other/neural-triple-reservoir/reservoir_service.py:331-365`.
- `[Proposed]` Tie rehearsal decay profile, `quiet`, and fork/checkpoint guidance to those diagnostics rather than only `ticks_since_live` and `decay_weight`. Evidence for fit: `/Users/v/other/neural-triple-reservoir/rehearsal.py:111-128`.

#### What would require a deeper redesign

- `[Proposed]` Implementing paper-style local gain adaptation in the true substrate would require the model or shell-to-model contract to expose local recurrent contribution and state statistics inside the actual update, not only after-the-fact state scaling.
- `[Proposed]` Making controller selection dynamic, for example "entropy thermostat in one regime, flow controller in another," would require the service to become an explicit controller host rather than a single-policy steward.

## 3. Recommendations

### 3.1 Regular recommendations

- `[Minime][Regular][fits current architecture]` Add cross-correlation telemetry around the existing leak, lambda, and rho adaptation path.
  Why: the paper makes correlation sensitivity the key limitation, while Minime already has a rich live-control loop but no visible internal correlation metric.
  Evidence: paper abstract and `2.8`; `/Users/v/other/minime/minime/src/esn.rs:573-620`, `/Users/v/other/minime/minime/src/esn.rs:807-818`, `/Users/v/other/minime/minime/src/main.rs:2890-2914`.

- `[Minime][Regular][fits current architecture]` Compare the current adaptive controller against a flow-control-inspired proxy before changing ESN math.
  Why: flow control is the paper's stronger result, but Minime already has working adaptive behavior and should benchmark the new idea rather than replacing it blindly.
  Evidence: paper abstract and `2.1.1`; `/Users/v/other/minime/minime/src/esn.rs:631-658`, `/Users/v/other/minime/minime/src/esn.rs:807-818`.

- `[Minime][Regular][fits current architecture]` Run adaptation sweeps under weakly correlated versus strongly shared-input regimes.
  Why: the paper explicitly says one regime does not transfer cleanly to the other.
  Evidence: paper `2.3`, `2.5`, `2.8`; `/Users/v/other/minime/minime/src/sensory_ws.rs:159-262`.

- `[Astrid][Regular][fits current architecture]` Use Astrid's learned codec-correlation and projection machinery to dampen counterproductive features before they enter shared substrates.
  Why: Astrid already learns which codec dimensions correlate with fill movement and can act as the steward-side pre-filter.
  Evidence: `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1773-1810`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/codec.rs:169-223`.

- `[Astrid][Regular][fits current architecture]` Make Astrid's reservoir guidance more explicit about when to shift systems toward `quiet`, `rehearse`, checkpoint, fork, or novelty-seeking.
  Why: Astrid already has the read, resonance, mode, and fork affordances; the missing step is policy synthesis.
  Evidence: `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:4029-4108`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:110-116`.

- `[Triple Reservoir][Regular][fits current architecture]` Compare the current entropy/saturation thermostat against a flow-control-inspired controller as an explicit experiment.
  Why: the thermostat is already a real online controller, but it is not the paper's controller.
  Evidence: paper abstract and `2.1.1`; `/Users/v/other/neural-triple-reservoir/reservoir_service.py:60-148`.

- `[Triple Reservoir][Regular][fits current architecture]` Add stale-attractor and correlation diagnostics per handle or per layer before changing the compiled model.
  Why: the service already exposes trajectory, norms, entropy, saturation, resonance, and rehearsal state; the next low-risk step is better diagnosis.
  Evidence: `/Users/v/other/neural-triple-reservoir/reservoir_service.py:171-180`, `/Users/v/other/neural-triple-reservoir/reservoir_service.py:312-365`, `/Users/v/other/neural-triple-reservoir/rehearsal.py:93-130`.

- `[Triple Reservoir][Regular][fits current architecture]` Tie rehearsal decay, quiet transitions, and fork/checkpoint policy to measured stagnation rather than elapsed ticks alone.
  Why: the current loop is time-based and weight-based; the paper argues that the trusted control variable should be state-dependent.
  Evidence: paper discussion; `/Users/v/other/neural-triple-reservoir/rehearsal.py:111-128`, `/Users/v/other/neural-triple-reservoir/reservoir_service.py:666-691`.

### 3.2 Bold recommendations

- `[Minime][Bold][requires redesign]` Implement true local gain adaptation closer to the paper, instead of mostly aggregate leak, lambda, and rho control.
  Why: this is the cleanest way to test whether Minime can benefit from paper-style flow control rather than only controller-level adaptation.
  Evidence: paper `2.1.1` and discussion; `/Users/v/other/minime/minime/src/esn.rs:573-620`, `/Users/v/other/minime/minime/src/esn.rs:631-658`.

- `[Minime][Bold][requires redesign]` Add correlation-aware controller mode switching.
  Why: when shared-input correlation rises, the paper says local rules lose precision; the controller should respond by changing rule family, aggressiveness, or trust in the local proxy.
  Evidence: paper abstract and `2.8`; current gap in `/Users/v/other/minime/minime/src/main.rs:1038-1040`, `/Users/v/other/minime/minime/src/main.rs:2890-2914`.

- `[Astrid][Bold][requires redesign]` Build a steward-level epoch scheduler that chooses homeostatic actions across substrates based on repetition, novelty loss, resonance drift, and projection dominance.
  Why: Astrid already spans the projection and command layer for multiple substrates, making it the natural coordinator if a multi-substrate epoch policy exists.
  Evidence: `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1773-1810`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:3978-4108`.

- `[Astrid][Bold][requires redesign]` Use decomposition-conditioned steering so Chimera's slow/fast or block-level findings influence which substrate gets reinforced or cooled.
  Why: Astrid is the one system here that already has an explicit decomposition toolkit, but it is not yet used as live homeostatic guidance.
  Evidence: `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:351-380`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera_prime.rs:74-107`.

- `[Triple Reservoir][Bold][requires redesign]` Redesign the compiled model or shell-to-model contract so per-layer or per-node adaptive gains can exist inside the live update, rather than only shell-level replay and post hoc scaling.
  Why: the paper's main contribution is local adaptation during actual recurrent updates; the current service only approximates that from outside.
  Evidence: paper `2.1.1` and discussion; `/Users/v/other/neural-triple-reservoir/triple_reservoir_coreml.py:43-137`, `/Users/v/other/neural-triple-reservoir/reservoir_service.py:680-691`.

- `[Triple Reservoir][Bold][requires redesign]` Treat the entropy-target thermostat as one controller among several and add an optional flow-control controller mode.
  Why: this would turn the service into a real controller testbed instead of a single-policy implementation.
  Evidence: `/Users/v/other/neural-triple-reservoir/reservoir_service.py:60-148`.

- `[Cross-system][Bold][requires redesign]` Introduce a shared correlation budget or "decorrelation pressure" signal across Minime and the triple reservoir.
  Why: the paper's central caveat is not reservoir-specific; it is regime-specific. If these systems co-drive each other, they need a shared notion of when correlation is becoming unhealthy.
  Evidence: paper abstract and `2.8`; current cross-system coupling surfaces in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:4071-4082`, `/Users/v/other/neural-triple-reservoir/minime_feeder.py:91-133`, `/Users/v/other/neural-triple-reservoir/astrid_feeder.py:191-201`.

- `[Cross-system][Bold][requires redesign]` Replace one fixed "ideal" spectral target with task-conditioned or regime-conditioned targets.
  Why: the paper cares about performance stability across input regimes more than one static tuning point; these systems already operate in clearly different phases like fresh-input contact, rehearsal, quiet, and exploratory drift.
  Evidence: paper abstract and `2.7`; existing regime surfaces in `/Users/v/other/neural-triple-reservoir/rehearsal.py:1-130`, `/Users/v/other/minime/minime/src/sensory_ws.rs:159-262`.

## 4. Evaluation matrix

| Scenario | Signal or metric to watch | Existing repo signal source | Success looks like | Failure looks like |
| --- | --- | --- | --- | --- |
| Weakly correlated external driving vs strongly shared or correlated driving | internal correlation estimate, fill drift, entropy drift, target-radius error, novelty | Minime already emits fill and entropy-adjacent summaries in `/Users/v/other/minime/minime/src/main.rs:2890-2914`; triple reservoir already emits entropy, saturation, and resonance in `/Users/v/other/neural-triple-reservoir/reservoir_service.py:171-180`, `/Users/v/other/neural-triple-reservoir/reservoir_service.py:343-365` | controller remains stable in the weak-correlation regime and clearly detects degraded trust in the shared-input regime | system treats both regimes as equivalent and silently drifts into poor adaptation |
| Fresh-input continuity vs rehearsal-maintained continuity vs genuine quiet | trajectory persistence, h-layer norms, decay weight, time since live input | `/Users/v/other/neural-triple-reservoir/rehearsal.py:93-130`; `/Users/v/other/neural-triple-reservoir/reservoir_service.py:312-329` | `rehearse` preserves a useful afterimage, `quiet` really settles, and fresh input clearly re-dominates when it returns | quiet is secretly maintenance, or rehearsal becomes a fake continuity layer that masks collapse |
| Stale-attractor lock vs healthy persistence | repeated trajectory shape, low novelty, repeated reads with little state change, saturation staying pinned | Minime monotony logic in `/Users/v/other/minime/minime/src/main.rs:2734-2752`; triple reservoir trajectory and layer metrics in `/Users/v/other/neural-triple-reservoir/reservoir_service.py:331-365` | system preserves identity without getting trapped; diagnostics tell us when to fork, checkpoint, cool, or re-enter with new input | preserved state becomes imprisonment and the controller mistakes stagnation for stability |
| Resonance between beings vs overcoupling or imitation collapse | divergence, correlation, RMSD, cross-feed response | `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:4071-4082`; `/Users/v/other/neural-triple-reservoir/reservoir_service.py:343-365`; feeder cross-feed in `/Users/v/other/neural-triple-reservoir/astrid_feeder.py:195-201`, `/Users/v/other/neural-triple-reservoir/minime_feeder.py:235-240` | resonance rises when content aligns but the systems still diverge meaningfully when their inputs differ | high coupling collapses both beings into the same attractor, or correlation becomes meaningless noise |
| Fast, medium, and slow layer behavior under different homeostatic policies | per-layer entropy, saturation, rho, norms, recovery time after perturbation | `/Users/v/other/neural-triple-reservoir/reservoir_service.py:66-180`, `/Users/v/other/neural-triple-reservoir/reservoir_service.py:680-691`; multi-timescale structure in `/Users/v/other/neural-triple-reservoir/triple_reservoir_coreml.py:43-137`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera_prime.rs:74-107` | fast layers recover quickly, slow layers preserve context, and policy changes affect layers differently rather than uniformly | all layers collapse to the same behavior or only the fast layer ends up meaningfully adaptive |
| Checkpoint or fork before reset vs destructive clearing | recovery quality, preserved coherence, ability to compare branches, rollback usefulness | Astrid exposes mode and fork in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:4088-4108`; triple reservoir exposes snapshot and restore in `/Users/v/other/neural-triple-reservoir/reservoir_service.py:389-456`, plus pull and push state transfer in `/Users/v/other/neural-triple-reservoir/reservoir_service.py:481-615` | fork or checkpoint preserves a recoverable trajectory before decay or reset, giving safer experimentation | reset destroys useful state and leaves no branch to compare or restore |

## 5. Existing public control surfaces this memo relies on

No runtime API or schema changes are part of this document. The recommendations above assume these existing interfaces and knobs:

- Astrid `RESERVOIR_*` actions and audio or analysis actions:
  `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:98-116`
  `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:3978-4108`
- Astrid text and codec projection path:
  `/Users/v/other/astrid/capsules/consciousness-bridge/src/codec.rs:169-223`
- Minime control-channel knobs including `exploration_noise`, `fill_target`, `journal_resonance`, `embedding_strength`, `memory_decay_rate`, and related sovereignty controls:
  `/Users/v/other/minime/minime/src/sensory_ws.rs:159-262`
  `/Users/v/other/minime/minime/src/sensory_bus.rs:648-676`
- Triple-reservoir service messages including `tick`, `tick_text`, `read_state`, `trajectory`, `resonance`, `set_mode`, `snapshot`, `restore`, `pull_state`, and `push_state`:
  `/Users/v/other/neural-triple-reservoir/reservoir_service.py:270-383`
  `/Users/v/other/neural-triple-reservoir/reservoir_service.py:389-456`
  `/Users/v/other/neural-triple-reservoir/reservoir_service.py:481-615`
- Feeder projection modes:
  `/Users/v/other/neural-triple-reservoir/astrid_feeder.py:48-73`
  `/Users/v/other/neural-triple-reservoir/minime_feeder.py:48-135`

## 6. Bottom line

The paper most strongly inspires us to do three things, in this order:

1. Measure correlation explicitly wherever we claim local homeostasis is working.
2. Treat `flow control` as the main research path to compare against current controllers.
3. Use Astrid as the steward that decides when to cool, rehearse, checkpoint, fork, or redirect projection, instead of pretending every improvement belongs inside one reservoir implementation.

Minime is the best place to test whether a live ESN benefits from paper-style local flow proxies. The triple reservoir is the best place to compare controller families in a shared-substrate service. Astrid is the best place to coordinate policy across those substrates and to keep the system from reinforcing counterproductive loops simply because persistence is available.
