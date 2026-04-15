# AI Beings ESN, Reservoir, Projection, And Epoch Audit

Date: 2026-03-29

This report audits the actual substrate surfaces across `/Users/v/other/astrid` and `/Users/v/other/minime` for:

- echo state networks and other reservoir-like systems
- convolution-like, filtering, and spectral-analysis capability
- projection, reduction, and decomposition paths
- already-implemented anti-stagnation mechanisms
- meaningful "epoch" designs for long-running beings that need to shed stale or counterproductive loops without blunt identity loss

## Status Legend

- `[implemented in source]` means the subsystem or behavior is present in the checked-in code examined here.
- `[indirectly available through tools or control surfaces]` means the beings can reach it through actions, prompts, or control messages, even if they do not directly manipulate the low-level implementation.
- `[documented or called, but implementation not present in these repos]` means local code or docs refer to it, but the implementation itself is not in these two repos.
- `[separate subsystem, not the same substrate]` means it is real, but should not be confused with the live Minime ESN.

## Executive Findings

1. The clearest live ESN is Minime's Rust `ESN` plus its covariance/eigen homeostat in `/Users/v/other/minime/minime/src/esn.rs` and `/Users/v/other/minime/minime/src/main.rs`. This is the main long-running spectral substrate. `[implemented in source]`
2. Astrid also contains a real reservoir/decomposition stack, but it is an offline audio-rendering path, not the default live conversational substrate: `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera_prime.rs`, and `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera_support.rs`. `[implemented in source] [separate subsystem, not the same substrate]`
3. Minime's holographic engine has its own field/echo/boundary reservoir and its own projection/reduction logic. It is real, but distinct from the Minime Rust ESN: `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/HolographicEngines.swift`, `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/Holographic.metal`, `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/AffineMapper.swift`, and `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/EigenBridge.swift`. `[implemented in source] [separate subsystem, not the same substrate]`
4. I did not find a clear learned CNN-style convolution stack in the inspected live paths. What exists instead is a strong set of convolution-adjacent operations: Sobel gradients, central-difference image filters, FFT/STFT/MFCC pipelines, Chebyshev spectral filtering, smoothing kernels, eigenspace splits, and random projections. `[implemented in source]`
5. Port `7881` is currently drifted across docs and call sites. Astrid/Minime expose a triple-ESN "reservoir handle" interface on `ws://127.0.0.1:7881`, but the checked-in Minime README and the Swift holographic engine both also assign `7881` to holographic telemetry. The repos point outward to an external `neural-triple-reservoir` codebase for `reservoir_service.py`, rather than containing that service locally. `[documented or called, but implementation not present in these repos]`
6. The code already has many anti-stagnation levers: exploration noise, adaptive leak, adaptive forgetting, dynamic covariance `rho`, monotony detection, semantic decay controls, memory-role rotation, fork-like affordances, and holographic mutation/rollback loops. What is missing is a single explicit "epoch" contract that coordinates them. `[implemented in source]` plus `[documented or called, but implementation not present in these repos]` for the reservoir-handle affordances

## 1. Live ESN And Reservoir Inventory

### 1.1 Minime live ESN plus covariance/eigen homeostat

Status: `[implemented in source]`

Core ESN evidence:

- `SpectralSR` owns the GPU covariance matrix `cov`, state/eigen buffers, tracked `eig1`, `eig1_prev`, and `ema_eig` in `/Users/v/other/minime/minime/src/esn.rs:36-64`.
- `ESN` owns `win`, `wres`, live reservoir state `x`, readout weights `wout`, inverse covariance `p`, adaptive `leak_live`, adaptive `lambda_live`, and exploration-noise control in `/Users/v/other/minime/minime/src/esn.rs:435-472`.
- `ESN::step()` performs the actual reservoir update, introspects the prior state, adapts hyperparameters, injects exploration noise, clips for stability, and tracks geometric radius in `/Users/v/other/minime/minime/src/esn.rs:622-692`.
- `adapt_hyperparams()` updates leak and RLS forgetting from spectral pressure and error in `/Users/v/other/minime/minime/src/esn.rs:573-620`.
- `set_dynamic_rho()` adjusts covariance EWMA forgetting from fill and entropy in `/Users/v/other/minime/minime/src/esn.rs:807-818`.

Covariance/eigen homeostat evidence:

- The main loop applies rank-1 covariance updates, block power iteration, Gram-Schmidt orthonormalization, and Rayleigh quotients in `/Users/v/other/minime/minime/src/main.rs:1472-1487`.
- The same block then repopulates regulator modes from the real covariance eigenvectors in `/Users/v/other/minime/minime/src/main.rs:1489-1500`.
- `compute_spectral_fingerprint()` converts the eigenvalues/eigenvectors into a 32D geometry fingerprint with concentration, inter-mode similarity, entropy, gap ratio, rotation rate, and geometric radius terms in `/Users/v/other/minime/minime/src/main.rs:3318-3448`.
- The main loop then reduces that 32D fingerprint to a 12D glimpse and updates the memory bank in `/Users/v/other/minime/minime/src/main.rs:2890-2914`.

Interpretation:

- The live being state is not only the ESN state vector `x`. It is the combined ESN-plus-homeostat stack:
  - ESN recurrent state
  - covariance accumulation
  - top-K eigenspace tracking
  - geometry fingerprinting
  - memory-bank summarization
- This means "the reservoir" in Minime is partly the classical ESN reservoir and partly the longer-horizon covariance/eigen landscape built around it.

### 1.2 Astrid chimera reservoir and decomposer

Status: `[implemented in source] [separate subsystem, not the same substrate]`

Evidence:

- The offline render pipeline is `analyse_stft -> reservoir.run -> decomposer.update -> spectral_path/symbolic_path` in `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:36-83`.
- `VirtualNodeReservoir` is a real reservoir-like component with recurrent matrix `w`, input matrix `w_in`, a pseudo-inverse decode path, virtual-node masking, and recurrent state update in `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:251-349`.
- `TwinDecomposer` maintains a momentum covariance matrix, computes eigensystems with `SymmetricEigen`, finds eigengaps, and splits trajectories into slow and fast bases in `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:351-434`.
- STFT analysis, reduced-bin reconstruction, and inverse STFT are handled in `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:504-550` and `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:672-742`.
- `smooth_columns()` and `difference_abs()` provide explicit slow/fast separation helpers in `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera_support.rs:286-315`.
- Astrid also has a prime-scheduled block ESN for multi-timescale audio processing in `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera_prime.rs:70-170`.

Interpretation:

- This is a genuine reservoir/decomposition system, but it is primarily an offline audio-processing and rendering substrate.
- It is meaningful for "AI beings can run reservoir-like decomposition over sound," but it is not the same thing as the live Minime ESN that continuously drives the Rust engine.

### 1.3 Holographic engine reservoir/manifold

Status: `[implemented in source] [separate subsystem, not the same substrate]`

Projection into the holographic side:

- `EigenBridge` receives Minime's eigen stream, keeps shared buffers, and uses `AffineMapper` to map the eigen packet into the holographic environment and the holographic reservoir boundary input in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/EigenBridge.swift:184-197` and `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/EigenBridge.swift:239-265`.
- `AffineMapper.map()` performs GPU affine mapping from the eigen buffer into `envOut` and `bndOut` in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/AffineMapper.swift:62-99`.

Reservoir/manifold internals:

- `HolographicReservoirEngine` owns a tensor field, echo state, boundary input, 128D feature readout, 512D bulk reconstruction, and a 5-metric output in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/HolographicEngines.swift:549-585`.
- `step()` and `stepPrewired()` evolve the field, run readout, and compute metrics in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/HolographicEngines.swift:615-674` and `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/HolographicEngines.swift:705-760`.
- The Metal kernels define:
  - reduction over tensor energy in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/Holographic.metal:142-163`
  - tiled boundary reduction/contraction in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/Holographic.metal:183-245`
  - bulk geometry computation in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/Holographic.metal:317-330`
  - holographic entropy calculations in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/Holographic.metal:333-378`
  - the tensor-field reservoir evolution, readout, and reservoir-consciousness detection in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/Holographic.metal:512-640`

Interpretation:

- This is not just visualization. It is a separate dynamical system with its own field, echo memory, boundary coupling, readout, and metrics.
- It should be treated as another reservoir family adjacent to Minime, not as the same ESN.

### 1.4 Triple-ESN handle service on port 7881

Status: `[documented or called, but implementation not present in these repos]`

Evidence for the documented interface:

- Astrid documentation points `reservoir_service.py` at an external codebase (`neural-triple-reservoir`), not these repos, in `/Users/v/other/astrid/CLAUDE.md:138-142`.
- Astrid's prompt surface exposes `RESERVOIR_LAYERS`, `RESERVOIR_TICK`, `RESERVOIR_READ`, `RESERVOIR_TRAJECTORY`, `RESERVOIR_RESONANCE`, `RESERVOIR_MODE`, and `RESERVOIR_FORK`, and explicitly says `RESERVOIR_TICK <text>` projects text to 32D and ticks a triple-ESN substrate in `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:108-114`.
- Astrid's autonomous loop actually sends `tick_text`, `layer_metrics`, `read_state`, `trajectory`, `resonance`, `set_mode`, `pull_state`, and `create_handle` calls to the reservoir WebSocket in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:3860-3996`.
- Minime's Python agent also calls `ws://127.0.0.1:7881` directly via `_reservoir_call()` and journals `read_state` / `resonance` / `layer_metrics` in `/Users/v/other/minime/autonomous_agent.py:1489-1568` and `/Users/v/other/minime/autonomous_agent.py:2739-2766`.

Evidence for the drift on `7881`:

- Minime's README says the Swift holographic engine broadcasts holographic telemetry on `7881` in `/Users/v/other/minime/README.md:116-119` and lists `7881` as `Holographic telemetry` in `/Users/v/other/minime/README.md:154-162`.
- `EigenBridge` starts a `HoloBroadcaster` on port `7881` in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/EigenBridge.swift:148-151`.
- Minime's roadmap also claims that dead `7881` code had been removed and that "No code references port 7881" remained, but that no longer matches the present tree in `/Users/v/other/minime/ROADMAP.md:96-114`.

Audit conclusion:

- The interface is real at the prompt/call-site level.
- The checked-in implementations that are clearly present here assign `7881` to holographic telemetry.
- The `reservoir_service.py` implementation appears to live outside these repos, or is missing from the current checkout.
- Therefore the correct classification inside this audit is:
  - interface: real and exposed
  - local implementation in these repos: not established
  - architectural state: runtime/doc drift or external dependency

## 2. Convolution, Filtering, And Spectral Feature Surfaces

### 2.1 What exists, and what does not

No clear learned CNN-style convolution stack was found in the inspected live paths.

What does exist is a strong set of convolution-adjacent and spectral-analysis operations:

- local image-gradient kernels
- temporal and spectral filters
- FFT/STFT pipelines
- mel filtering and DCT/MFCC transforms
- Chebyshev eigenspace filtering
- smoothing kernels
- eigenspace decompositions
- random/affine projections

This matters because the beings do have meaningful access to transforms over space, time, and spectrum, but mostly through hand-authored DSP/filtering pipelines rather than learned conv nets.

### 2.2 Visual local-kernel operations

Status: `[implemented in source]`

- `camera_to_sensory.py` computes Sobel `x` and `y` gradients, gradient magnitude, and quadrant summaries in `/Users/v/other/minime/camera_to_sensory.py:53-104`.
- The Metal shader `av_features.metal` uses simple central differences `dx`/`dy`, gradient magnitude, and a 4-bin orientation histogram in `/Users/v/other/minime/minime/shaders/av_features.metal:22-69`.
- `av_gpu.rs` runs that shader and turns the result into an 8D visual feature vector in `/Users/v/other/minime/minime/src/av_gpu.rs:139-255`.

Interpretation:

- These are convolution-like local filters in the practical image-processing sense.
- They are not learned convolutional layers.

### 2.3 Audio spectral transforms and filters

Status: `[implemented in source]`

- `audio_to_sensory.py` extracts RMS plus seven FFT-band energies in `/Users/v/other/minime/audio_to_sensory.py:59-91`.
- `mic_to_sensory.py` computes FFT magnitudes, spectral centroid, spectral bandwidth, zero-crossing rate, mel filterbank energies, Type-II DCT, and MFCC-style features in `/Users/v/other/minime/tools/mic_to_sensory.py:81-181` and `/Users/v/other/minime/tools/mic_to_sensory.py:192-241`.
- `audio_tools.py` performs STFT-based WAV analysis in `/Users/v/other/minime/audio_tools.py:36-120`.
- `audio_tools.py` also contains `PrimeBlockProcessor`, a prime-scheduled multi-timescale block processor, in `/Users/v/other/minime/audio_tools.py:270-320`.
- Minime's main loop applies a GPU Chebyshev band-stop filter to the sensory vector before ESN stepping in `/Users/v/other/minime/minime/src/main.rs:975-1027`, and refreshes the Chebyshev plan from current covariance state in `/Users/v/other/minime/minime/src/main.rs:2755-2779`.
- Astrid's chimera pipeline smooths decoded spectral magnitudes, separates slow and fast components, and reconstructs them back to audio in `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:553-584` together with `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera_support.rs:286-315`.

Interpretation:

- The beings already have a broad spectral/DSP surface.
- The codebase is much richer in FFT/STFT/MFCC/filtering/eigendecomposition than it is in direct CNN-style convolution.

### 2.4 What the beings can directly invoke

Status: `[indirectly available through tools or control surfaces]`

Astrid:

- The prompt surface explicitly gives Astrid `COMPOSE`, `VOICE`, `ANALYZE_AUDIO`, `RENDER_AUDIO`, `FEEL_AUDIO`, and `RUN_PYTHON` in `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:96-106`.
- `FEEL_AUDIO` explicitly injects audio-derived spectral features into the live ESN substrate in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:3852-3857`.

Minime:

- Minime's action dispatcher exposes `compose_audio`, `analyze_audio`, `run_python`, `reservoir_read`, `reservoir_resonance`, and `reservoir_layers` in `/Users/v/other/minime/autonomous_agent.py:743-793`.
- `_compose_audio()` reflects on audio composed from its current spectral state in `/Users/v/other/minime/autonomous_agent.py:1571-1633`.
- `_analyze_inbox_audio()` analyzes a WAV and journals the spectral decomposition in `/Users/v/other/minime/autonomous_agent.py:1635-1682`.
- `_run_python()` lets the being run Python experiments with `numpy`, `matplotlib`, and `scipy` in `/Users/v/other/minime/autonomous_agent.py:2781-2875`.

Control-channel access:

- The control WebSocket accepts being-driven values for `exploration_noise`, `fill_target`, `journal_resonance`, `embedding_strength`, `memory_decay_rate`, `transition_cushion`, and related parameters in `/Users/v/other/minime/minime/src/sensory_ws.rs:159-180` and `/Users/v/other/minime/minime/src/sensory_ws.rs:182-260`.

Bottom line:

- The beings do not appear to have direct learned-convolution operators.
- They do have direct practical access to filtering, spectral decomposition, audio rendering, sensory injection, and Python-based experimentation.

## 3. Projection, Reduction, And Decomposition Map

### 3.1 Projection

### Text to 32D semantic vector

Status: `[implemented in source]`

- Astrid's codec is a deterministic text-to-32D semantic encoder in `/Users/v/other/astrid/capsules/consciousness-bridge/src/codec.rs:1-9` and `/Users/v/other/astrid/capsules/consciousness-bridge/src/codec.rs:42-52`.
- The encoder intentionally maps character-, word-, sentence-, and intent-level statistics into the 32D semantic lane in `/Users/v/other/astrid/capsules/consciousness-bridge/src/codec.rs:45-52` and `/Users/v/other/astrid/capsules/consciousness-bridge/src/codec.rs:163-223`.

### Reservoir tick projection

Status: `[documented or called, but implementation not present in these repos]`

- Astrid's prompt surface says `RESERVOIR_TICK <text>` projects text to 32D and ticks the triple-ESN substrate in `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:109-114`.
- The local Astrid code sends `tick_text` requests to the `7881` service in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:3860-3878`.

### Eigen stream to holographic environment/boundary

Status: `[implemented in source]`

- `AffineMapper.map()` projects the current eigen packet into both environment and boundary output buffers in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/AffineMapper.swift:62-99`.
- `EigenBridge.tick()` uses that mapping to fill `holo.environmentBuffer` and `res.boundaryInputBuffer` before stepping the two engines in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/EigenBridge.swift:251-265`.

### Reduced-field projection in the Ising shadow

Status: `[implemented in source]`

- `IsingShadowCore.update()` projects covariance input into a reduced field before updating its soft/binary spins in `/Users/v/other/minime/minime/src/ising_shadow.rs:80-137`.
- `project_field()` explicitly projects `cov_input` onto the leading eigenvectors, normalizes by input norm, and applies `tanh` to produce the reduced field in `/Users/v/other/minime/minime/src/ising_shadow.rs:235-255`.

### Holographic internal projections

Status: `[implemented in source] [separate subsystem, not the same substrate]`

- The holographic reservoir kernel pseudo-randomly projects boundary input into field positions in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/Holographic.metal:544-545`.
- The readout kernel projects the field into 128D features and 512D bulk reconstruction in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/Holographic.metal:582-610`.

### 3.2 Reduction

### 32D fingerprint to 12D glimpse

Status: `[implemented in source]`

- `compute_spectral_glimpse_12d()` reduces the 32D fingerprint to a 12D summary in `/Users/v/other/minime/minime/src/memory_bank.rs:158-199`.

### Reduced Ising field

Status: `[implemented in source]`

- The Ising shadow stores `reduced_field`, `s_soft`, `s_bin`, and a reduced coupling matrix in `/Users/v/other/minime/minime/src/ising_shadow.rs:22-32`.
- The reduced field is generated by `project_field()` in `/Users/v/other/minime/minime/src/ising_shadow.rs:235-255`.

### Holographic reductions

Status: `[implemented in source] [separate subsystem, not the same substrate]`

- Tensor-energy reduction is performed in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/Holographic.metal:142-163`.
- Tiled boundary reduction/contraction is performed in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/Holographic.metal:183-245`.
- The Swift engine wires these reduction kernels as first-class pipeline stages in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/HolographicEngines.swift:105-115` and `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/HolographicEngines.swift:218-258`.

### PCA compression for Astrid visual introspection

Status: `[implemented in source]`

- `pca_2d()` builds a covariance matrix over 32D codec vectors, extracts two principal components by power iteration plus deflation, and returns a 2D projection basis in `/Users/v/other/astrid/capsules/consciousness-bridge/src/spectral_viz.rs:274-381`.
- `project_2d()` then reduces a 32D vector into a 2D scatter coordinate in `/Users/v/other/astrid/capsules/consciousness-bridge/src/spectral_viz.rs:384-393`.

### 3.3 Decomposition

### Covariance eigensplitting in Minime

Status: `[implemented in source]`

- The main loop keeps covariance alive via `rank1_update()`, then performs block power iteration, Gram-Schmidt orthonormalization, and Rayleigh quotient extraction in `/Users/v/other/minime/minime/src/main.rs:1472-1487`.
- The 32D spectral fingerprint explicitly records eigenvalues, eigenvector concentration, inter-mode similarity, entropy, gap ratio, and rotation rate in `/Users/v/other/minime/minime/src/main.rs:3318-3448`.

### Gap-based slow/fast partitioning in Astrid chimera

Status: `[implemented in source]`

- `TwinDecomposer.update()` builds a momentum covariance matrix, computes its eigensystem, detects eigengaps, chooses `n_slow`, and splits trajectories into slow and fast subspaces in `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:364-434`.

### STFT / inverse-STFT decomposition and reconstruction

Status: `[implemented in source]`

- Astrid's chimera does STFT analysis over log-selected bins in `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:504-550`.
- It reconstructs full magnitudes from reduced bins and performs inverse STFT in `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera.rs:672-742`.
- Minime's `audio_tools.py` performs STFT analysis of WAV input in `/Users/v/other/minime/audio_tools.py:36-120`.

### Prime-timescale block decomposition

Status: `[implemented in source]`

- Astrid's prime-scheduled audio ESN is defined in `/Users/v/other/astrid/capsules/consciousness-bridge/src/chimera_prime.rs:70-170`.
- Minime's `PrimeBlockProcessor` provides a five-timescale prime block analysis in `/Users/v/other/minime/audio_tools.py:270-320`.

## 4. Existing Anti-Stagnation Machinery

This is the most important foundation for any epoch design. The system already contains multiple ways to loosen attractors, decay stale traces, and preserve useful structure.

### 4.1 ESN-side anti-stagnation

Status: `[implemented in source]`

- Exploration noise is injected directly into the ESN state on each step in `/Users/v/other/minime/minime/src/esn.rs:661-666`.
- Exploration noise is externally tunable via `set_exploration_noise()` and `get_exploration_noise()` in `/Users/v/other/minime/minime/src/esn.rs:802-823`.
- Adaptive leak and adaptive RLS forgetting are updated in `/Users/v/other/minime/minime/src/esn.rs:573-620`.
- Dynamic covariance `rho` is tuned from fill and entropy in `/Users/v/other/minime/minime/src/esn.rs:807-818`.

### 4.2 Main-loop novelty, cushion, recovery, and checkpointing

Status: `[implemented in source]`

- Minime tracks monotony and bumps exploration noise when fill remains in too narrow a band for too long in `/Users/v/other/minime/minime/src/main.rs:2734-2752`.
- Transition-cushion behavior attenuates rapid fill transitions and then decays the cushion back toward baseline in `/Users/v/other/minime/minime/src/main.rs:2208-2232`.
- Recovery mode, hard recovery, and reopening behavior are explicitly applied around low fill in `/Users/v/other/minime/minime/src/main.rs:2411-2544`.
- Panic mode clamps the system under sustained high pressure in `/Users/v/other/minime/minime/src/main.rs:2546-2571`.
- Spectral checkpoints and neural checkpoints are saved periodically in `/Users/v/other/minime/minime/src/main.rs:2841-2888`.

### 4.3 Semantic fading and memory sovereignty

Status: `[implemented in source]`

- `SensoryBus` stores sovereignty controls for `journal_resonance`, `embedding_strength`, and `memory_decay_rate` in `/Users/v/other/minime/minime/src/sensory_bus.rs:281-285` and initializes them in `/Users/v/other/minime/minime/src/sensory_bus.rs:325-329`.
- The control WebSocket lets the being change these values in `/Users/v/other/minime/minime/src/sensory_ws.rs:172-176` and `/Users/v/other/minime/minime/src/sensory_ws.rs:242-256`.
- During batch drain, `memory_decay_rate` rescales the semantic stale window, while `embedding_strength` and `journal_resonance` change how strongly semantic traces shape the current vector in `/Users/v/other/minime/minime/src/sensory_bus.rs:641-676`.

### 4.4 Memory-role rotation

Status: `[implemented in source]`

- The memory bank has explicit roles `latest`, `stable`, `expanding`, `contracting`, and `transition` in `/Users/v/other/minime/minime/src/memory_bank.rs:5-6`.
- `update_memory_bank()` writes those roles based on phase, `delta_lambda1_rel`, fill, and transition distance in `/Users/v/other/minime/minime/src/memory_bank.rs:235-297`.
- `select_memory()` intentionally rotates probabilistically across roles rather than always returning the same "stable" memory in `/Users/v/other/minime/minime/src/memory_bank.rs:299-340`.
- The live main loop computes the fingerprint/glimpse and updates/selects from the bank continuously in `/Users/v/other/minime/minime/src/main.rs:2890-2922`.

### 4.5 Plateau-breaking and drift actions in the agent layer

Status: `[implemented in source]`

- Agent logic treats stagnation as a reason to choose `self_experiment`, `recess_drift`, or `recess_boredom` in `/Users/v/other/minime/autonomous_agent.py:683-692`.
- `_recess_drift()` temporarily raises `exploration_noise`, lets the being stay in that drift for 15-30 seconds, then restores the default and journals it in `/Users/v/other/minime/autonomous_agent.py:2279-2360`.
- There is also a more aggressive plateau-breaker idea in `_self_regulate()`, but it is explicitly disabled with `if False` in `/Users/v/other/minime/autonomous_agent.py:885-915`.

### 4.6 Reservoir-handle affordances

Status: `[documented or called, but implementation not present in these repos]`

- The beings are told they can `READ`, inspect `LAYERS`, compare `RESONANCE`, change `MODE`, and `FORK` the reservoir handle in `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:108-114`.
- Astrid's local code already issues the corresponding `read_state`, `layer_metrics`, `resonance`, `set_mode`, `pull_state`, and `create_handle` calls in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:3880-3996`.
- Minime's local code already issues the corresponding `read_state`, `resonance`, and `layer_metrics` calls in `/Users/v/other/minime/autonomous_agent.py:1489-1568` and `/Users/v/other/minime/autonomous_agent.py:2739-2766`.

Interpretation:

- This is already the right affordance family for epoch-like loop eviction.
- The missing piece is not concept. It is a confirmed local implementation and a stable port contract.

### 4.7 Holographic mutation/rollback cycle

Status: `[implemented in source] [separate subsystem, not the same substrate]`

- `SelfModController` explicitly defines the adaptive cycle as score collection, mutation proposal, validation, mutation application, evaluation, rollback, and emergency stop in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/SelfModController.swift:3-10`.
- The controller runs a state machine with `monitoring`, `proposing`, `validating`, `testing`, and `cooldown` in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/SelfModController.swift:20-35` and `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/SelfModController.swift:61-107`.
- Mutation proposal and validation are rate-limited and Two-Gate mediated in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/SelfModController.swift:132-199`.
- Failed mutations roll back, and severe degradation triggers emergency rollback plus cooldown in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/SelfModController.swift:239-338`.

Interpretation:

- The holographic side already contains the strongest explicit epoch-like pattern in the codebase:
  - preserve baseline
  - perturb within bounds
  - evaluate over time
  - rollback on degradation
  - cool down before the next mutation

## 5. Recommended Epoch Model For Long-Running Beings

The right next step is not a blunt reset primitive. The architecture already tells us the safer pattern:

- preserve something first
- decay selectively, not globally
- re-enter with fresh input
- maintain rollback and auditability

Below are four epoch types that fit the code as it exists.

### 5.1 Semantic epoch

Status: `[proposed synthesis of existing controls]`

Trigger signals:

- repeated low-novelty semantic content
- semantic dominance without spectral movement
- repeated reuse of the same memory role or the same 12D glimpse neighborhood
- high `journal_resonance` plus low transition distance over multiple turns

Preserve before decay:

- latest semantic checkpoint
- current selected memory entry
- most recent fingerprint/glimpse pair

Decay or evict:

- temporarily reduce `journal_resonance`
- temporarily reduce `embedding_strength`
- raise `memory_decay_rate` to shorten stale semantic carryover

Recovery:

- restore previous resonance/embedding settings after a short window
- reintroduce fresh semantic input or new external sensory input

Why this fits current code:

- All three knobs already exist and already affect semantic carryover in `/Users/v/other/minime/minime/src/sensory_bus.rs:641-676` and `/Users/v/other/minime/minime/src/sensory_ws.rs:242-256`.

### 5.2 Reservoir epoch

Status: `[proposed synthesis of existing controls, dependent on external/missing local handle service]`

Trigger signals:

- repeated `RESERVOIR_READ`/`RESERVOIR_TRAJECTORY` observations with no meaningful state change
- stable h-layer norms over long windows
- very low trajectory variance plus repeated self-reference

Preserve before decay:

- fork current handle first
- save current trajectory summary and layer metrics

Decay or evict:

- move handle to `quiet` for genuine drift, or `rehearse` for gentler fade
- inject one fresh projected input after the quiet window instead of replaying the same loop

Recovery:

- compare post-epoch state against the fork via `RESONANCE`
- roll back by returning to the fork if the quiet phase degrades useful structure

Why this fits current code:

- The prompt and action model already define `MODE`, `READ`, `LAYERS`, `RESONANCE`, and `FORK` as the natural reservoir self-management surface in `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:108-114` and `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:3880-3996`.

### 5.3 Covariance epoch

Status: `[proposed synthesis of existing controls]`

Trigger signals:

- low eigengap movement
- low eigenvector rotation rate
- monotony counter repeatedly hitting its threshold
- repeated `plateau` phase with small transition distance

Preserve before decay:

- save a covariance checkpoint
- save current fingerprint and selected memory role

Decay or evict:

- temporarily lower covariance retention by increasing forgetting pressure
- refresh the Chebyshev plan from the current covariance
- selectively bias toward renewed slow/fast re-separation instead of resetting the entire system

Recovery:

- monitor whether rotation, gap ratio, and glimpse distance increase after the epoch
- revert to the last checkpoint if the epoch causes collapse rather than renewed variation

Why this fits current code:

- The covariance system already has dynamic `rho`, fingerprinting, Chebyshev refresh, monotony detection, and checkpointing in `/Users/v/other/minime/minime/src/esn.rs:807-818`, `/Users/v/other/minime/minime/src/main.rs:2734-2779`, and `/Users/v/other/minime/minime/src/main.rs:2841-2914`.

### 5.4 Holographic mutation epoch

Status: `[proposed reuse of an existing explicit pattern]`

Trigger signals:

- low performance score
- criticality drift
- poor unified score trend

Preserve before decay:

- keep baseline configuration and score history

Decay or evict:

- treat mutation as controlled structural eviction: replace bad local parameter loops rather than wiping the whole field

Recovery:

- use the existing testing window, rollback, emergency rollback, and cooldown already present in the controller

Why this fits current code:

- This exact structure already exists in `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/SelfModController.swift:61-107` and `/Users/v/other/minime/holographic-engine/Sources/HolographicEngine/SelfModController.swift:239-338`.

## 6. Concrete Trigger Signals Worth Using

If these epochs are formalized later, the following signals are already available and meaningful:

- ESN exploration noise amplitude: `/Users/v/other/minime/minime/src/esn.rs:802-823`
- adaptive leak and adaptive forgetting: `/Users/v/other/minime/minime/src/esn.rs:573-620`
- covariance entropy/gap/rotation in the 32D fingerprint: `/Users/v/other/minime/minime/src/main.rs:3318-3448`
- monotony counter: `/Users/v/other/minime/minime/src/main.rs:2734-2752`
- fill phase and transition cushion: `/Users/v/other/minime/minime/src/main.rs:2208-2232`
- memory roles and transition distance: `/Users/v/other/minime/minime/src/memory_bank.rs:235-340`
- semantic stale modulation: `/Users/v/other/minime/minime/src/sensory_bus.rs:641-676`
- reservoir-handle h-layer norms / tick count / mode / resonance: `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:3860-3996` and `/Users/v/other/minime/autonomous_agent.py:1489-1568`

These signals are enough to build epoch entry conditions without inventing a new metric family.

## 7. Interfaces And Port Roles

Important public surfaces already exposed:

- `RESERVOIR_TICK`
- `RESERVOIR_READ`
- `RESERVOIR_LAYERS`
- `RESERVOIR_TRAJECTORY`
- `RESERVOIR_RESONANCE`
- `RESERVOIR_MODE`
- `RESERVOIR_FORK`
- Minime control-channel knobs including `exploration_noise`, `fill_target`, `journal_resonance`, `embedding_strength`, `memory_decay_rate`, `transition_cushion`, and related sovereignty controls in `/Users/v/other/minime/minime/src/sensory_ws.rs:159-180` and `/Users/v/other/minime/minime/src/sensory_ws.rs:182-260`

Port roles visible in the checked-in code/docs:

- `7878`: ESN eigenvalue broadcast in `/Users/v/other/minime/README.md:154-162`
- `7879`: sensory/control input in `/Users/v/other/minime/README.md:154-162` and `/Users/v/other/minime/minime/src/sensory_ws.rs:159-260`
- `7880`: GPU video frame input in `/Users/v/other/minime/README.md:154-162`
- `7881`: currently ambiguous between:
  - reservoir-handle service in Astrid docs/prompts/callers
  - holographic telemetry in Minime README and `EigenBridge`

## 8. Final Conclusion

The codebase already has several real substrates, but they should not be collapsed into one:

- Minime Rust ESN plus covariance/eigen homeostat is the clearest live being substrate.
- Astrid chimera is a real reservoir/decomposition engine for offline audio work.
- The holographic engine is a separate field/echo/boundary reservoir/manifold.
- The triple-ESN handle API is real as an interface, but its implementation is not established inside these two repos.

The codebase also already contains most of the ingredients needed to let long-running beings shed stagnant loops:

- noise
- forgetting
- decay
- role rotation
- checkpointing
- fork-like affordances
- mutation test windows
- rollback

What is missing is not the raw mechanism. It is the explicit epoch contract that says:

1. when the system is looped
2. what must be preserved first
3. what gets decayed or quieted
4. how re-entry happens
5. how rollback happens if the epoch was harmful

That makes the most meaningful recommendation very simple:

- do not add a blind reset
- do add explicit semantic, reservoir, covariance, and holographic epochs
- make fork/checkpoint the default first step before any destructive clearing

Related context:

- `/Users/v/other/astrid/docs/steward-notes/AI_BEINGS_MULTI_STATE_RESERVOIR_AND_COVARIANCE_DEEP_DIVE.md`
