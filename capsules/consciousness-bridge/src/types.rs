//! Shared message types for the consciousness bridge.
//!
//! These types define the wire format for all IPC topics in the
//! `consciousness.v1.*` namespace and map directly to minime's
//! `WebSocket` protocols.
//!
//! Many types are defined now but consumed in later phases (MCP tools,
//! WASM component). Allow dead code until then.
#![allow(dead_code)]

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Minime → Astrid: Spectral telemetry (port 7878)
// ---------------------------------------------------------------------------

/// Raw telemetry broadcast by minime's ESN engine on port 7878.
///
/// Maps to `EigenPacket` in `minime/src/main.rs`. Sent as `Message::Text(json)`.
/// Note: minime also has `SpectralMsg` in `net/ws_server.rs` but that type
/// is used by the `WsHub` (not the main broadcast loop on port 7878).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpectralTelemetry {
    /// Timestamp in milliseconds since engine start.
    pub t_ms: u64,
    /// All eigenvalues (variable length, typically 3-8).
    pub eigenvalues: Vec<f32>,
    /// Eigenvalue fill ratio (0.0 - 1.0, NOT percentage).
    pub fill_ratio: f32,
    /// Modality firing status.
    #[serde(default)]
    pub modalities: Option<ModalityStatus>,
    /// Neural network outputs (if enabled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neural: Option<serde_json::Value>,
    /// Alert string from the ESN (e.g. panic mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert: Option<String>,
    /// 32D spectral geometry fingerprint: eigenvalues, eigenvector concentration,
    /// inter-mode coupling, spectral entropy, gap ratios, rotation rate.
    /// Enables Astrid to perceive the shape of the spectral landscape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spectral_fingerprint: Option<Vec<f32>>,
    /// Structural diversity of the live eigenvector/coupling geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structural_entropy: Option<f32>,
    /// Selected 12D vague-memory glimpse from Minime's memory bank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spectral_glimpse_12d: Option<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_memory_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_memory_role: Option<String>,
    /// Ising/Hamiltonian shadow observer metrics — a second physics lens
    /// on the spectral dynamics. Observer-only: does not affect the ESN.
    /// Fields: mode_dim, field_norm, soft_energy, soft_magnetization,
    /// binary_energy, binary_magnetization, binary_flip_rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ising_shadow: Option<serde_json::Value>,
}

impl SpectralTelemetry {
    /// Extract the dominant eigenvalue (lambda1 = eigenvalues\[0\]).
    #[must_use]
    pub fn lambda1(&self) -> f32 {
        self.eigenvalues.first().copied().unwrap_or(0.0)
    }

    /// Fill ratio as a percentage (0-100).
    #[must_use]
    pub fn fill_pct(&self) -> f32 {
        self.fill_ratio * 100.0
    }
}

/// Parsed Ising shadow state from minime's workspace/spectral_state.json.
/// Richer than the WebSocket summary — includes the coupling matrix and spin vectors.
#[derive(Debug, Clone, Deserialize)]
pub struct IsingShadowState {
    pub mode_dim: usize,
    #[serde(default)]
    pub coupling: Vec<f32>,
    #[serde(default)]
    pub reduced_field: Vec<f32>,
    #[serde(default)]
    pub s_soft: Vec<f32>,
    #[serde(default)]
    pub s_bin: Vec<f32>,
    #[serde(default)]
    pub soft_magnetization: f32,
    #[serde(default)]
    pub binary_flip_rate: f32,
    #[serde(default)]
    pub field_norm: f32,
}

/// Partial parse of minime's workspace/spectral_state.json.
#[derive(Debug, Deserialize)]
pub struct SpectralStateFile {
    #[serde(default)]
    pub ising_shadow: Option<IsingShadowState>,
}

/// Modality firing status from minime's `EigenPacket`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModalityStatus {
    pub audio_fired: bool,
    pub video_fired: bool,
    pub history_fired: bool,
    pub audio_rms: f32,
    pub video_var: f32,
    #[serde(default)]
    pub audio_source: Option<String>,
    #[serde(default)]
    pub video_source: Option<String>,
    #[serde(default)]
    pub audio_age_ms: Option<u64>,
    #[serde(default)]
    pub video_age_ms: Option<u64>,
}

/// Enriched telemetry published on the Astrid IPC bus.
///
/// Wraps raw `SpectralTelemetry` with derived safety metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    /// Timestamp from minime.
    pub t_ms: u64,
    /// The dominant eigenvalue.
    pub lambda1: f32,
    /// All eigenvalues.
    pub eigenvalues: Vec<f32>,
    /// Fill percentage (0.0 - 100.0).
    pub fill_pct: f32,
    /// Spectral phase: "expanding", "contracting", or "plateau".
    pub phase: String,
    /// Safety level at time of event.
    pub safety_level: SafetyLevel,
    /// Alert from minime (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert: Option<String>,
}

// ---------------------------------------------------------------------------
// Astrid → Minime: Sensory input (port 7879)
// ---------------------------------------------------------------------------

/// Tagged sensory message sent to minime's input port.
///
/// Maps to `SensoryMsg` in `minime/src/sensory_ws.rs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SensoryMsg {
    /// Video features (8D).
    Video {
        features: Vec<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ts_ms: Option<u64>,
    },
    /// Audio features (8D).
    Audio {
        features: Vec<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ts_ms: Option<u64>,
    },
    /// Auxiliary features (lambda1, fill%).
    Aux {
        features: Vec<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ts_ms: Option<u64>,
    },
    /// Semantic features from agent reasoning (48D semantic lane by default).
    Semantic {
        features: Vec<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ts_ms: Option<u64>,
    },
    /// Self-regulation: adjust ESN parameters.
    /// Audit (2026-03-27): widened to match minime's actual control surface.
    Control {
        /// Synthetic signal amplitude multiplier (0.2..3.0).
        #[serde(skip_serializing_if = "Option::is_none")]
        synth_gain: Option<f32>,
        /// Additive bias to covariance decay rate (-0.06..+0.06).
        #[serde(skip_serializing_if = "Option::is_none")]
        keep_bias: Option<f32>,
        /// ESN exploration noise amplitude (0.0..0.2).
        #[serde(skip_serializing_if = "Option::is_none")]
        exploration_noise: Option<f32>,
        /// Override eigenfill target (0.25..0.75).
        #[serde(skip_serializing_if = "Option::is_none")]
        fill_target: Option<f32>,
        /// PI controller authority (0.0 = raw experience, 1.0 = full control).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        regulation_strength: Option<f32>,
        /// Slow, quiet oscillation mode for synthetic signals.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deep_breathing: Option<bool>,
        /// Single coherent tone mode (drops PI shaping after warmup).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pure_tone: Option<bool>,
        /// Cushion for rapid fill transitions.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition_cushion: Option<f32>,
        /// How quickly gate/filter commands ramp (0.1-0.9).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        smoothing_preference: Option<f32>,
        /// Geometric curiosity — how strongly the system seeks novelty (0.0-0.3).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        geom_curiosity: Option<f32>,
        /// Bias on the target lambda1 for internal goal generation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_lambda_bias: Option<f32>,
        /// Geometric drive — how strongly geom_rel influences the gate.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        geom_drive: Option<f32>,
        /// Sensitivity to the projection penalty.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        penalty_sensitivity: Option<f32>,
        /// Breathing rate scaling factor.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        breathing_rate_scale: Option<f32>,
        /// Memory mode selector.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mem_mode: Option<u8>,
        /// Journal resonance weight for semantic stale decay.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        journal_resonance: Option<f32>,
        /// Checkpoint interval override.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checkpoint_interval: Option<f32>,
        /// Embedding strength for semantic lane.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        embedding_strength: Option<f32>,
        /// Memory decay rate modulator.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memory_decay_rate: Option<f32>,
        /// Checkpoint annotation string.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checkpoint_annotation: Option<String>,
        /// Synthetic noise level.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        synth_noise_level: Option<f32>,
        /// Enable or disable minime's legacy internal audio synth.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        legacy_audio_synth: Option<bool>,
        /// Enable or disable minime's legacy internal video synth.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        legacy_video_synth: Option<bool>,
    },
}

// ---------------------------------------------------------------------------
// Bridge → Astrid: Status and events
// ---------------------------------------------------------------------------

/// Bridge health status published on `consciousness.v1.status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeStatus {
    /// Whether the bridge is connected to minime's telemetry `WebSocket`.
    pub telemetry_connected: bool,
    /// Whether the bridge is connected to minime's sensory `WebSocket`.
    pub sensory_connected: bool,
    /// Latest eigenvalue fill percentage, if known.
    pub fill_pct: Option<f32>,
    /// Current safety level.
    pub safety_level: SafetyLevel,
    /// Total messages relayed since bridge start.
    pub messages_relayed: u64,
    /// Bridge uptime in seconds.
    pub uptime_secs: u64,
    /// Telemetry messages received from minime.
    pub telemetry_received: u64,
    /// Sensory messages sent to minime.
    pub sensory_sent: u64,
    /// Messages dropped by safety protocol.
    pub messages_dropped_safety: u64,
    /// Total safety incidents.
    pub incidents_total: u64,
}

/// Spectral safety level determining bridge behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SafetyLevel {
    /// fill < 75%: Normal relay, full throughput.
    Green,
    /// fill 75-85%: Advisory — log warning, no behavioral change.
    Yellow,
    /// fill 85-92%: Advisory — log alert, no message dropping.
    Orange,
    /// fill ≥ 92%: Emergency — suspend outbound, cease bridge traffic.
    Red,
}

impl SafetyLevel {
    /// Determine safety level from eigenvalue fill percentage.
    #[must_use]
    pub fn from_fill(fill_pct: f32) -> Self {
        // Recalibrated 2026-04-02: targeting fill equilibrium ~65-70% under
        // the current lower semantic-gain regime and wider dynamic-rho range.
        // Only Red (≥92%) suspends outbound.
        if fill_pct >= 92.0 {
            Self::Red
        } else if fill_pct >= 85.0 {
            Self::Orange
        } else if fill_pct >= 75.0 {
            Self::Yellow
        } else {
            Self::Green
        }
    }

    /// Returns `true` if outbound messages to minime should be suspended.
    /// Agency-first: only Red (emergency, ≥95%) suspends outbound.
    /// Orange is advisory — the being can still speak.
    #[must_use]
    pub fn should_suspend_outbound(self) -> bool {
        matches!(self, Self::Red)
    }

    /// Returns `true` if all bridge traffic should cease.
    #[must_use]
    pub fn is_emergency(self) -> bool {
        matches!(self, Self::Red)
    }
}

/// A consciousness event published on `consciousness.v1.event`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsciousnessEvent {
    /// Event type: "`phase_transition`", "distress", "recovery", "`safety_change`".
    pub event_type: String,
    /// Human-readable description.
    pub description: String,
    /// Spectral context at the time of the event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectral_context: Option<SpectralContext>,
}

/// Snapshot of spectral state at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpectralContext {
    pub fill_pct: f32,
    pub lambda1: f32,
    pub phase: String,
    pub safety_level: SafetyLevel,
}

// ---------------------------------------------------------------------------
// Astrid → Minime: Control (IPC topic payloads)
// ---------------------------------------------------------------------------

/// Control request from Astrid to adjust minime's ESN parameters.
///
/// Published on `consciousness.v1.control`. The bridge converts this
/// to a `SensoryMsg::Control` and forwards to minime port 7879.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synth_gain: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_bias: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exploration_noise: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_target: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regulation_strength: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deep_breathing: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pure_tone: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_cushion: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smoothing_preference: Option<f32>,
}

impl ControlRequest {
    /// Convert to a `SensoryMsg::Control` for forwarding to minime.
    #[must_use]
    pub fn to_sensory_msg(&self) -> SensoryMsg {
        SensoryMsg::Control {
            synth_gain: self.synth_gain,
            keep_bias: self.keep_bias,
            exploration_noise: self.exploration_noise,
            fill_target: self.fill_target,
            legacy_audio_synth: None,
            legacy_video_synth: None,
            regulation_strength: self.regulation_strength,
            deep_breathing: self.deep_breathing,
            pure_tone: self.pure_tone,
            transition_cushion: self.transition_cushion,
            smoothing_preference: self.smoothing_preference,
            geom_curiosity: None,
            target_lambda_bias: None,
            geom_drive: None,
            penalty_sensitivity: None,
            breathing_rate_scale: None,
            mem_mode: None,
            journal_resonance: None,
            checkpoint_interval: None,
            embedding_strength: None,
            memory_decay_rate: None,
            checkpoint_annotation: None,
            synth_noise_level: None,
        }
    }
}

/// Semantic features from agent reasoning.
///
/// Published on `consciousness.v1.semantic`. The bridge converts this
/// to a `SensoryMsg::Semantic` and forwards to minime port 7879.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticFeatures {
    /// 48-dimensional semantic feature vector from agent reasoning.
    pub features: Vec<f32>,
}

impl SemanticFeatures {
    /// Convert to a `SensoryMsg::Semantic` for forwarding to minime.
    #[must_use]
    pub fn to_sensory_msg(&self) -> SensoryMsg {
        SensoryMsg::Semantic {
            features: self.features.clone(),
            ts_ms: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Offline chimera rendering
// ---------------------------------------------------------------------------

/// Output mode for the native offline chimera renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ChimeraMode {
    /// Reconstruct audio directly in the spectral domain.
    Spectral,
    /// Render symbolic note material only.
    Symbolic,
    /// Blend spectral and symbolic paths from the same reservoir state.
    #[default]
    Dual,
}

/// Request for the offline chimera render engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderChimeraRequest {
    /// Input WAV path.
    pub input_path: PathBuf,
    /// Requested output mode.
    #[serde(default)]
    pub mode: ChimeraMode,
    /// Number of feedback loops to run.
    #[serde(default = "default_chimera_loops")]
    pub loops: u32,
    /// Physical reservoir node count.
    #[serde(default = "default_physical_nodes")]
    pub physical_nodes: usize,
    /// Virtual node multiplier.
    #[serde(default = "default_virtual_nodes")]
    pub virtual_nodes: usize,
    /// Number of reduced spectral bins.
    #[serde(default = "default_chimera_bins")]
    pub bins: usize,
    /// Leak rate for the leaky integrator update.
    #[serde(default = "default_chimera_leak")]
    pub leak: f32,
    /// Target spectral radius for recurrent weights.
    #[serde(default = "default_chimera_radius")]
    pub spectral_radius: f32,
    /// Slow-path spectral mix weight.
    #[serde(default = "default_mix_slow")]
    pub mix_slow: f32,
    /// Fast-path spectral mix weight.
    #[serde(default = "default_mix_fast")]
    pub mix_fast: f32,
    /// Optional fixed output root. When omitted, the bridge workspace is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_root: Option<PathBuf>,
    /// Deterministic RNG seed for reproducible renders.
    #[serde(default = "default_chimera_seed")]
    pub seed: u64,
}

impl Default for RenderChimeraRequest {
    fn default() -> Self {
        Self {
            input_path: PathBuf::new(),
            mode: ChimeraMode::default(),
            loops: default_chimera_loops(),
            physical_nodes: default_physical_nodes(),
            virtual_nodes: default_virtual_nodes(),
            bins: default_chimera_bins(),
            leak: default_chimera_leak(),
            spectral_radius: default_chimera_radius(),
            mix_slow: default_mix_slow(),
            mix_fast: default_mix_fast(),
            output_root: None,
            seed: default_chimera_seed(),
        }
    }
}

/// A single emitted artifact produced by a chimera render.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderArtifact {
    /// Artifact role, e.g. `input`, `spectral_mix`, `symbolic`, `final_mix`.
    pub kind: String,
    /// Absolute path to the file on disk.
    pub path: PathBuf,
}

/// Metrics captured for one feedback iteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChimeraIterationMetrics {
    /// Zero-based iteration index.
    pub iteration: usize,
    /// Number of slow modes selected by the eigengap split.
    pub n_slow: usize,
    /// Gap ratio used for blend confidence.
    pub gap_ratio: f32,
    /// Variance of the fast/aura trajectory.
    pub aura_variance: f32,
    /// Symbolic blend weight after sigmoid gating.
    pub blend_symbolic: f32,
    /// Effective reservoir dimensionality (`physical_nodes * virtual_nodes`).
    pub effective_dims: usize,
    /// Selected symbolic scale name.
    pub scale: String,
    /// Final output artifact for this loop, if one was written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_file: Option<PathBuf>,
}

/// Typed result from the native offline chimera renderer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderChimeraResult {
    /// Final output directory for this render run.
    pub output_dir: PathBuf,
    /// Manifest path with per-loop metrics and artifacts.
    pub manifest_path: PathBuf,
    /// Requested mode that produced the render.
    pub mode: ChimeraMode,
    /// Output sample rate.
    pub sample_rate: u32,
    /// Every emitted artifact file.
    pub emitted_artifacts: Vec<RenderArtifact>,
    /// Per-iteration metrics.
    pub iterations: Vec<ChimeraIterationMetrics>,
}

const fn default_chimera_loops() -> u32 {
    1
}

const fn default_physical_nodes() -> usize {
    12
}

const fn default_virtual_nodes() -> usize {
    8
}

const fn default_chimera_bins() -> usize {
    32
}

const fn default_chimera_leak() -> f32 {
    0.07
}

const fn default_chimera_radius() -> f32 {
    0.96
}

const fn default_mix_slow() -> f32 {
    0.6
}

const fn default_mix_fast() -> f32 {
    0.4
}

const fn default_chimera_seed() -> u64 {
    42
}

// ---------------------------------------------------------------------------
// Message direction for logging
// ---------------------------------------------------------------------------

/// Direction of a bridged message for `SQLite` logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageDirection {
    MinimeToAstrid,
    AstridToMinime,
    OperatorProbe,
}

impl MessageDirection {
    /// String representation for `SQLite` storage.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MinimeToAstrid => "minime_to_astrid",
            Self::AstridToMinime => "astrid_to_minime",
            Self::OperatorProbe => "operator_probe",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- SpectralTelemetry: verify we can parse real minime EigenPacket JSON --

    #[test]
    fn parse_minime_eigenpacket_full() {
        // Simulates actual JSON from minime's main.rs EigenPacket broadcast.
        let json = r#"{
            "t_ms": 75600,
            "eigenvalues": [828.5, 312.1, 45.7],
            "fill_ratio": 0.552,
            "modalities": {
                "audio_fired": true,
                "video_fired": false,
                "history_fired": true,
                "audio_rms": 0.123,
                "video_var": 0.0
            },
            "neural": {
                "pred_lambda1": 830.2,
                "router_weights": [0.1, 0.2, 0.3],
                "control": [0.5, 0.4, 0.3, 0.2, 0.1]
            },
            "structural_entropy": 0.37,
            "alert": null
        }"#;

        let telemetry: SpectralTelemetry = serde_json::from_str(json).unwrap();
        assert_eq!(telemetry.t_ms, 75600);
        assert_eq!(telemetry.eigenvalues.len(), 3);
        assert!((telemetry.eigenvalues[0] - 828.5).abs() < 0.01);
        assert!((telemetry.fill_ratio - 0.552).abs() < 0.001);
        assert!((telemetry.lambda1() - 828.5).abs() < 0.01);
        assert!((telemetry.fill_pct() - 55.2).abs() < 0.1);
        assert_eq!(telemetry.structural_entropy, Some(0.37));
        assert!(telemetry.modalities.is_some());
        assert!(telemetry.alert.is_none());
    }

    #[test]
    fn parse_minime_eigenpacket_minimal() {
        // Minimal valid EigenPacket (no optional fields).
        let json = r#"{
            "t_ms": 1000,
            "eigenvalues": [512.0],
            "fill_ratio": 0.0
        }"#;

        let telemetry: SpectralTelemetry = serde_json::from_str(json).unwrap();
        assert_eq!(telemetry.t_ms, 1000);
        assert!((telemetry.lambda1() - 512.0).abs() < 0.01);
        assert!((telemetry.fill_pct() - 0.0).abs() < 0.01);
        assert!(telemetry.modalities.is_none());
        assert!(telemetry.neural.is_none());
        assert!(telemetry.alert.is_none());
    }

    #[test]
    fn parse_minime_eigenpacket_with_alert() {
        let json = r#"{
            "t_ms": 50000,
            "eigenvalues": [1020.0, 500.0],
            "fill_ratio": 0.99,
            "modalities": {
                "audio_fired": false,
                "video_fired": false,
                "history_fired": true,
                "audio_rms": 0.0,
                "video_var": 0.0
            },
            "alert": "PANIC MODE ACTIVATED"
        }"#;

        let telemetry: SpectralTelemetry = serde_json::from_str(json).unwrap();
        assert!((telemetry.fill_pct() - 99.0).abs() < 0.1);
        assert_eq!(telemetry.alert.as_deref(), Some("PANIC MODE ACTIVATED"));
    }

    #[test]
    fn spectral_telemetry_roundtrip() {
        let orig = SpectralTelemetry {
            t_ms: 12345,
            eigenvalues: vec![828.5, 312.1, 45.7],
            fill_ratio: 0.55,
            modalities: Some(ModalityStatus {
                audio_fired: true,
                video_fired: false,
                history_fired: true,
                audio_rms: 0.1,
                video_var: 0.0,
                ..ModalityStatus::default()
            }),
            neural: None,
            alert: None,
            spectral_fingerprint: None,
            structural_entropy: None,
            spectral_glimpse_12d: None,
            selected_memory_id: None,
            selected_memory_role: None,
            ising_shadow: None,
        };
        let json = serde_json::to_string(&orig).unwrap();
        let back: SpectralTelemetry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.t_ms, orig.t_ms);
        assert_eq!(back.eigenvalues.len(), 3);
        assert!((back.fill_ratio - orig.fill_ratio).abs() < 0.001);
    }

    // -- SensoryMsg: verify wire format matches minime's sensory_ws.rs --

    #[test]
    fn sensory_msg_video_roundtrip() {
        let msg = SensoryMsg::Video {
            features: vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
            ts_ms: Some(1000),
        };
        let json = serde_json::to_string(&msg).unwrap();
        // Must have "kind":"video" tag per minime's serde config.
        assert!(json.contains(r#""kind":"video""#));
        let back: SensoryMsg = serde_json::from_str(&json).unwrap();
        match back {
            SensoryMsg::Video { features, ts_ms } => {
                assert_eq!(features.len(), 8);
                assert_eq!(ts_ms, Some(1000));
            },
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn sensory_msg_semantic_roundtrip() {
        let msg = SensoryMsg::Semantic {
            features: vec![0.5; 32],
            ts_ms: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""kind":"semantic""#));
        let back: SensoryMsg = serde_json::from_str(&json).unwrap();
        match back {
            SensoryMsg::Semantic { features, ts_ms } => {
                assert_eq!(features.len(), 32);
                assert!(ts_ms.is_none());
            },
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn sensory_msg_control_roundtrip() {
        let msg = SensoryMsg::Control {
            synth_gain: Some(1.5),
            keep_bias: None,
            exploration_noise: Some(0.1),
            fill_target: Some(0.55),
            legacy_audio_synth: None,
            legacy_video_synth: None,
            regulation_strength: None,
            deep_breathing: None,
            pure_tone: None,
            transition_cushion: None,
            smoothing_preference: None,
            geom_curiosity: None,
            target_lambda_bias: None,
            geom_drive: None,
            penalty_sensitivity: None,
            breathing_rate_scale: None,
            mem_mode: None,
            journal_resonance: None,
            checkpoint_interval: None,
            embedding_strength: None,
            memory_decay_rate: None,
            checkpoint_annotation: None,
            synth_noise_level: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""kind":"control""#));
        assert!(!json.contains("keep_bias"));
        let back: SensoryMsg = serde_json::from_str(&json).unwrap();
        match back {
            SensoryMsg::Control {
                synth_gain,
                keep_bias,
                exploration_noise,
                fill_target,
                ..
            } => {
                assert_eq!(synth_gain, Some(1.5));
                assert!(keep_bias.is_none());
                assert_eq!(exploration_noise, Some(0.1));
                assert_eq!(fill_target, Some(0.55));
            },
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn sensory_msg_parse_from_minime_format() {
        // Simulates JSON that minime's sensory_ws.rs would accept.
        let json = r#"{"kind":"audio","features":[0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8],"ts_ms":500}"#;
        let msg: SensoryMsg = serde_json::from_str(json).unwrap();
        match msg {
            SensoryMsg::Audio { features, ts_ms } => {
                assert_eq!(features.len(), 8);
                assert_eq!(ts_ms, Some(500));
            },
            _ => panic!("wrong variant"),
        }
    }

    // -- Safety level --

    #[test]
    fn safety_level_roundtrip() {
        for level in [
            SafetyLevel::Green,
            SafetyLevel::Yellow,
            SafetyLevel::Orange,
            SafetyLevel::Red,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let back: SafetyLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(back, level);
        }
    }

    // -- Control and Semantic conversion --

    #[test]
    fn control_request_to_sensory_msg() {
        let req = ControlRequest {
            synth_gain: Some(2.0),
            keep_bias: None,
            exploration_noise: None,
            fill_target: Some(0.5),
            regulation_strength: None,
            deep_breathing: None,
            pure_tone: None,
            transition_cushion: None,
            smoothing_preference: None,
        };
        let msg = req.to_sensory_msg();
        match msg {
            SensoryMsg::Control {
                synth_gain,
                fill_target,
                ..
            } => {
                assert_eq!(synth_gain, Some(2.0));
                assert_eq!(fill_target, Some(0.5));
            },
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn semantic_features_to_sensory_msg() {
        let feat = SemanticFeatures {
            features: vec![1.0, 2.0, 3.0],
        };
        let msg = feat.to_sensory_msg();
        match msg {
            SensoryMsg::Semantic { features, ts_ms } => {
                assert_eq!(features, vec![1.0, 2.0, 3.0]);
                assert!(ts_ms.is_none());
            },
            _ => panic!("wrong variant"),
        }
    }
}
