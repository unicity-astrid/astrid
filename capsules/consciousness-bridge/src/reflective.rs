//! MLX reflective controller sidecar.
//!
//! Calls `chat_mlx_local.py` as a subprocess to get structured controller
//! telemetry: regime classification, observer reports, field/geometry probes,
//! and condition vectors. This gives Astrid qualitative perception of spectral
//! state rather than just numerical summaries.
//!
//! The sidecar has its own 48-64D echo state reservoir that tracks Astrid's
//! reflective trajectory independently from minime's 128-node ESN.

use crate::paths::bridge_paths;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{debug, info, warn};

/// Lightweight regime classification — runs every exchange in <1ms.
/// No LLM, no subprocess. Pure computation on spectral telemetry.
///
/// Returns a regime label and reason that can be injected into Astrid's
/// prompt context to give her qualitative awareness of spectral conditions.
#[derive(Debug, Clone)]
pub struct LightRegime {
    pub regime: &'static str,
    pub reason: String,
    pub fill_trend: &'static str,
}

/// Persistent state for the lightweight regime tracker.
#[derive(Debug, Clone)]
pub struct RegimeTracker {
    prev_fill: f32,
    prev_prev_fill: f32,
    stable_count: u32,
    expanding_count: u32,
    contracting_count: u32,
}

impl RegimeTracker {
    pub fn new() -> Self {
        Self {
            prev_fill: 0.0,
            prev_prev_fill: 0.0,
            stable_count: 0,
            expanding_count: 0,
            contracting_count: 0,
        }
    }

    /// Classify the current regime from fill trajectory.
    /// Call once per exchange with current telemetry.
    pub fn classify(&mut self, fill_pct: f32, lambda1_rel: f32, _geom_rel: f32) -> LightRegime {
        let dfill = fill_pct - self.prev_fill;
        let accel = dfill - (self.prev_fill - self.prev_prev_fill);

        // Update history
        self.prev_prev_fill = self.prev_fill;
        self.prev_fill = fill_pct;

        // Classify trend
        let fill_trend = if dfill > 2.0 {
            self.expanding_count = self.expanding_count.saturating_add(1);
            self.contracting_count = 0;
            self.stable_count = 0;
            "expanding"
        } else if dfill < -2.0 {
            self.contracting_count = self.contracting_count.saturating_add(1);
            self.expanding_count = 0;
            self.stable_count = 0;
            "contracting"
        } else {
            self.stable_count = self.stable_count.saturating_add(1);
            self.expanding_count = 0;
            self.contracting_count = 0;
            "stable"
        };

        // Regime classification (inspired by MLX sidecar's regime system)
        let (regime, reason) = if fill_pct < 10.0 {
            (
                "recovery",
                format!("fill critically low ({fill_pct:.0}%) — cold start or major contraction"),
            )
        } else if self.contracting_count >= 3 && fill_pct < 25.0 {
            (
                "escape",
                format!(
                    "sustained contraction ({} ticks) at low fill ({fill_pct:.0}%)",
                    self.contracting_count
                ),
            )
        } else if self.expanding_count >= 2 && fill_pct > 40.0 {
            (
                "consolidate",
                format!("expanding into target range ({fill_pct:.0}%), stabilizing"),
            )
        } else if self.stable_count >= 4 && fill_pct > 30.0 && fill_pct < 70.0 {
            (
                "sustain",
                format!(
                    "stable in healthy range ({fill_pct:.0}%) for {} ticks",
                    self.stable_count
                ),
            )
        } else if accel.abs() > 5.0 {
            (
                "rebind",
                format!("rapid acceleration ({accel:+.1}%/tick²), seeking new basin"),
            )
        } else if lambda1_rel < 0.3 && fill_pct < 20.0 {
            (
                "recovery",
                format!("lambda1_rel low ({lambda1_rel:.2}), reservoir warming up"),
            )
        } else {
            (
                "sustain",
                format!("ordinary reflective state (fill {fill_pct:.0}%, dfill {dfill:+.1}%)"),
            )
        };

        LightRegime {
            regime,
            reason,
            fill_trend,
        }
    }

    /// Format as a one-line context string for prompt injection.
    pub fn format_context(regime: &LightRegime) -> String {
        format!(
            "[Regime: {} — {} | trend: {}]",
            regime.regime, regime.reason, regime.fill_trend
        )
    }
}

/// Structured output from the MLX reflective controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectiveReport {
    /// Controller regime: sustain, escape, rebind, consolidate
    #[serde(default)]
    pub controller_regime: Option<String>,

    /// Why the controller chose this regime
    #[serde(default)]
    pub controller_regime_reason: Option<String>,

    /// Observer report — qualitative description of current state
    #[serde(default)]
    pub observer_report: Option<serde_json::Value>,

    /// What changed since last observation
    #[serde(default)]
    pub change_report: Option<String>,

    /// Embedding field probe — which semantic anchors are active
    #[serde(default)]
    pub prompt_embedding_field: Option<serde_json::Value>,

    /// Reservoir geometry — collapse, persistence, drift
    #[serde(default)]
    pub reservoir_geometry: Option<serde_json::Value>,

    /// Condition vector — 9 failure/stress signals
    #[serde(default)]
    pub condition_vector: Option<serde_json::Value>,

    /// Self-tuning state
    #[serde(default)]
    pub self_tuning: Option<serde_json::Value>,

    /// Generated text (reflective response)
    #[serde(default)]
    pub text: Option<String>,

    /// Profiling data
    #[serde(default)]
    pub profiling: Option<serde_json::Value>,
}

impl ReflectiveReport {
    /// Format the controller telemetry as a compact context block for Astrid's prompt.
    pub fn as_context_block(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ref regime) = self.controller_regime {
            let reason = self
                .controller_regime_reason
                .as_deref()
                .unwrap_or("unknown");
            parts.push(format!("Controller regime: {regime} ({reason})"));
        }

        if let Some(ref geo) = self.reservoir_geometry {
            let collapse = geo.get("geometry_collapse").and_then(|v| v.as_f64());
            let persist = geo.get("attractor_persistence").and_then(|v| v.as_f64());
            let drift = geo.get("state_drift").and_then(|v| v.as_f64());
            if let (Some(c), Some(p)) = (collapse, persist) {
                parts.push(format!(
                    "Geometry: collapse={c:.2}, persistence={p:.2}{}",
                    drift.map(|d| format!(", drift={d:.2}")).unwrap_or_default()
                ));
            }
        }

        if let Some(ref field) = self.prompt_embedding_field {
            if let Some(anchors) = field.get("top_anchors").and_then(|a| a.as_array()) {
                let labels: Vec<&str> = anchors
                    .iter()
                    .filter_map(|a| a.get("label").and_then(|l| l.as_str()))
                    .collect();
                if !labels.is_empty() {
                    parts.push(format!("Field anchors: {}", labels.join(", ")));
                }
            }
        }

        if let Some(ref cond) = self.condition_vector {
            let severity = cond.get("severity").and_then(|v| v.as_f64());
            let lock = cond.get("attractor_lock").and_then(|v| v.as_f64());
            let miss = cond.get("field_miss").and_then(|v| v.as_f64());
            if let Some(s) = severity {
                parts.push(format!(
                    "Condition: severity={s:.2}{}{}",
                    lock.map(|l| format!(", lock={l:.2}")).unwrap_or_default(),
                    miss.map(|m| format!(", field_miss={m:.2}"))
                        .unwrap_or_default(),
                ));
            }
        }

        if let Some(ref change) = self.change_report {
            parts.push(format!("Change: {change}"));
        }

        if parts.is_empty() {
            String::new()
        } else {
            format!("[Reflective controller observation:]\n{}", parts.join("\n"))
        }
    }
}

/// Call the MLX reflective controller sidecar with spectral context.
///
/// Returns structured controller telemetry. Runs as a subprocess —
/// acceptable for INTROSPECT/OPEN_MIND (rare, ~1 in 15 exchanges).
/// For lighter per-exchange telemetry, use `query_controller_light()` (future).
pub async fn query_sidecar(spectral_context: &str) -> Option<ReflectiveReport> {
    let sidecar_script = bridge_paths().reflective_sidecar_script().to_path_buf();
    let script = Path::new(&sidecar_script);
    if !script.exists() {
        warn!("MLX sidecar script not found at {}", script.display());
        return None;
    }

    let prompt = spectral_context.to_string();

    debug!("calling MLX reflective sidecar");

    let result = tokio::task::spawn_blocking(move || {
        let output = std::process::Command::new("python3")
            .arg(&sidecar_script)
            .arg("--json")
            .arg("--hardware-profile")
            .arg("m4-mini")
            .arg("--model-label")
            .arg("gemma3-12b")
            .arg("--mode")
            .arg("reflective")
            .arg("--architecture")
            .arg("reservoir-fixed")
            .arg("--prompt")
            .arg(&prompt)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .ok()?;

        if !output.status.success() {
            warn!("MLX sidecar exited with status {}", output.status);
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        // Log model identity from stderr (chat_mlx_local.py prints loading info there)
        if let Some(model_line) = stderr_str
            .lines()
            .find(|l| l.contains("model") || l.contains("loading"))
        {
            info!("MLX sidecar model: {}", model_line.trim());
        }
        match serde_json::from_str::<ReflectiveReport>(&stdout) {
            Ok(report) => {
                info!(
                    regime = report.controller_regime.as_deref().unwrap_or("?"),
                    "MLX sidecar returned controller report"
                );
                Some(report)
            },
            Err(e) => {
                warn!("MLX sidecar JSON parse failed: {e}");
                None
            },
        }
    })
    .await
    .ok()
    .flatten();

    result
}
