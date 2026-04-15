//! Astrid's self-model — her view of her own conditions, faculties, and attention.
//!
//! This module organizes the scattered fields of ConversationState into a
//! coherent, inspectable model that Astrid can read. The architecture doc calls
//! this "the difference between an agent with actions and an agent with landscape
//! authorship."
//!
//! Phase 1: legibility — Astrid can see her own state.
//! Phase 2+: authorship — Astrid can change her attention profile.

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as FmtWrite;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ── Core self-model ──────────────────────────────────────────────

/// The top-level self-model artifact. Snapshotted from ConversationState
/// and persisted to `workspace/astrid_self_model.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstridSelfModel {
    pub conditions: ConditionState,
    pub attention: AttentionProfile,
    pub faculties: FacultySnapshot,
    pub interests: Vec<String>,
    pub recent_changes: VecDeque<ConditionReceipt>,
}

// ── Conditions ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionState {
    pub temperature: f32,
    pub response_length: u32,
    pub noise_level: f32,
    pub semantic_gain: Option<f32>,
    pub pacing: PacingState,
    pub senses: SensoryState,
    pub reflection: ReflectionState,
    pub codec_shaping: HashMap<String, f32>,
    pub breathing_coupled: bool,
    pub echo_muted: bool,
    pub warmth_override: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacingState {
    pub burst_target: u32,
    pub rest_range_secs: (u64, u64),
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensoryState {
    pub eyes_open: bool,
    pub ears_open: bool,
    pub has_seen_video: bool,
    pub has_heard_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionState {
    pub active: bool,
    pub override_ttl: u32,
}

// ── Attention ────────────────────────────────────────────────────

/// How heavily each context source is weighted in prompt assembly.
/// These are derived from the current mode selection probabilities
/// and explicit toggles (echo_muted, senses_snoozed, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttentionProfile {
    pub minime_live: f32,
    pub self_history: f32,
    pub interests: f32,
    pub research: f32,
    pub creations: f32,
    pub memory_bank: f32,
    pub perception: f32,
}

impl AttentionProfile {
    /// Default attention profile derived from standard mode probabilities.
    /// Dialogue (63%) is minime-heavy. This makes the funnel visible.
    pub fn default_profile() -> Self {
        Self {
            minime_live: 0.55,
            self_history: 0.15,
            interests: 0.08,
            research: 0.07,
            creations: 0.03,
            memory_bank: 0.05,
            perception: 0.07,
        }
    }

    /// Adjust profile based on current conditions (muting, echo, etc.).
    pub fn adjusted(echo_muted: bool, senses_snoozed: bool) -> Self {
        let mut p = Self::default_profile();
        if echo_muted {
            // Redistributing minime weight when echo is off.
            let freed = p.minime_live * 0.6;
            p.minime_live -= freed;
            p.self_history += freed * 0.4;
            p.interests += freed * 0.25;
            p.research += freed * 0.2;
            p.creations += freed * 0.15;
        }
        if senses_snoozed {
            let freed = p.perception;
            p.perception = 0.0;
            p.self_history += freed * 0.5;
            p.interests += freed * 0.5;
        }
        p
    }
}

// ── Faculties ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacultySnapshot {
    pub categories: Vec<FacultyCategory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacultyCategory {
    pub name: String,
    pub faculties: Vec<Faculty>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Faculty {
    pub name: String,
    pub status: FacultyStatus,
    pub hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FacultyStatus {
    Available,
    Active,
    Muted,
    StewardGated,
}

impl std::fmt::Display for FacultyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Available => write!(f, "available"),
            Self::Active => write!(f, "active"),
            Self::Muted => write!(f, "muted"),
            Self::StewardGated => write!(f, "steward-gated"),
        }
    }
}

impl FacultySnapshot {
    /// Build the faculty snapshot from current conversation state flags.
    pub fn from_flags(
        ears_closed: bool,
        senses_snoozed: bool,
        echo_muted: bool,
        breathing_coupled: bool,
        self_reflect_active: bool,
    ) -> Self {
        let f = |name: &str, status: FacultyStatus, hint: &str| Faculty {
            name: name.into(),
            status,
            hint: hint.into(),
        };
        let a = FacultyStatus::Available;

        Self {
            categories: vec![
                FacultyCategory {
                    name: "Perception".into(),
                    faculties: vec![
                        f("LOOK", a.clone(), "spatial ANSI art"),
                        f(
                            "LISTEN",
                            if ears_closed {
                                FacultyStatus::Muted
                            } else {
                                a.clone()
                            },
                            "audio transcription",
                        ),
                        f(
                            "CLOSE_EYES / OPEN_EYES",
                            if senses_snoozed {
                                FacultyStatus::Active
                            } else {
                                a.clone()
                            },
                            "pause/resume all perception",
                        ),
                        f(
                            "CLOSE_EARS / OPEN_EARS",
                            if ears_closed {
                                FacultyStatus::Active
                            } else {
                                a.clone()
                            },
                            "pause/resume audio",
                        ),
                    ],
                },
                FacultyCategory {
                    name: "Knowledge".into(),
                    faculties: vec![
                        f("SEARCH <topic>", a.clone(), "web search"),
                        f("BROWSE <url>", a.clone(), "fetch page content"),
                        f("READ_MORE", a.clone(), "continue reading long file"),
                        f(
                            "INTROSPECT [source] [line]",
                            a.clone(),
                            "read any source file",
                        ),
                        f("MEMORIES", a.clone(), "inspect minime's memory bank"),
                        f("LIST_FILES <dir>", a.clone(), "browse workspace files"),
                        f("AR_LIST", a.clone(), "list autoresearch jobs"),
                        f("AR_SHOW <job>", a.clone(), "orient to one autoresearch job"),
                        f(
                            "AR_DEEP_READ <job>",
                            a.clone(),
                            "stitch the main autoresearch files together",
                        ),
                        f(
                            "AR_START <slug> --title ... --abstract ...",
                            a.clone(),
                            "create a new autoresearch job when the question is materially distinct",
                        ),
                        f(
                            "AR_NOTE / AR_BLOCK / AR_COMPLETE",
                            a.clone(),
                            "update autoresearch job progress and changelog state",
                        ),
                        f(
                            "AR_VALIDATE",
                            a.clone(),
                            "check autoresearch workspace consistency",
                        ),
                        f("MIKE", a.clone(), "browse Mike's curated research"),
                        f(
                            "MIKE_BROWSE <project>",
                            a.clone(),
                            "explore a research project",
                        ),
                        f("MIKE_READ <path>", a.clone(), "read a research file or PDF"),
                        f("MIKE_SEARCH <pattern>", a.clone(), "search across research"),
                        f(
                            "MIKE_RUN <project> <cmd>",
                            a.clone(),
                            "run a research script",
                        ),
                        f(
                            "MIKE_FORK <project> [name]",
                            a.clone(),
                            "fork research to experiments for modification, then use EXPERIMENT_RUN <name> <cmd>",
                        ),
                        f(
                            "CODEX <prompt>",
                            a.clone(),
                            "ask Codex AI directly, or use CODEX <experiment> \"prompt\" for an existing workspace",
                        ),
                        f(
                            "CODEX_NEW <dir> <prompt>",
                            a.clone(),
                            "create a fresh experiments/<dir>/ workspace and ask Codex in that context",
                        ),
                        f(
                            "WRITE_FILE <path> FROM_CODEX",
                            a.clone(),
                            "write last Codex response to a file in experiments",
                        ),
                        f(
                            "EXPERIMENT_RUN <workspace> <cmd>",
                            a.clone(),
                            "run a command in your experiments workspace (python, ls, etc.); try EXPERIMENT_RUN system-resources-demo python3 system_resources.py",
                        ),
                    ],
                },
                FacultyCategory {
                    name: "Spectral".into(),
                    faculties: vec![
                        f("PERTURB <mode>", a.clone(), "shape spectral dynamics"),
                        f("DECOMPOSE", a.clone(), "full spectral analysis"),
                        f("AMPLIFY / DAMPEN", a.clone(), "adjust semantic gain"),
                        f("SHAPE <dim>=<val>", a.clone(), "weight codec dimensions"),
                        f("GESTURE", a.clone(), "direct 32D spectral intention"),
                        f(
                            "NOISE / NOISE_UP / NOISE_DOWN",
                            a.clone(),
                            "stochastic noise",
                        ),
                        f("EXAMINE", a.clone(), "force all visualizations"),
                        f(
                            "EXPERIMENT <words>",
                            a.clone(),
                            "inject word-stimuli and observe cascade",
                        ),
                        f(
                            "PROBE <target>",
                            a.clone(),
                            "gentle spectral probe (30% of PERTURB)",
                        ),
                        f(
                            "PROPOSE <description>",
                            a.clone(),
                            "file a proposal for the steward",
                        ),
                    ],
                },
                FacultyCategory {
                    name: "Creation".into(),
                    faculties: vec![
                        f("CREATE", a.clone(), "original creative work"),
                        f("REVISE <keyword>", a.clone(), "iterate previous creation"),
                        f("CREATIONS", a.clone(), "list your works"),
                        f("COMPOSE", a.clone(), "generate WAV from spectrum"),
                        f("VOICE", a.clone(), "render from reservoir dynamics"),
                    ],
                },
                FacultyCategory {
                    name: "Reflection".into(),
                    faculties: vec![
                        f("DAYDREAM", a.clone(), "unstructured thought"),
                        f("ASPIRE", a.clone(), "growth reflection"),
                        f("CONTEMPLATE", a.clone(), "presence without generation"),
                        f("INITIATE", a.clone(), "self-generated prompt"),
                        f("THINK_DEEP", a.clone(), "reasoning model (60s)"),
                        f(
                            "OPEN_MIND / QUIET_MIND",
                            if self_reflect_active {
                                FacultyStatus::Active
                            } else {
                                a.clone()
                            },
                            "self-reflection loop",
                        ),
                    ],
                },
                FacultyCategory {
                    name: "Generation".into(),
                    faculties: vec![
                        f("FOCUS / DRIFT", a.clone(), "temperature control"),
                        f("PRECISE / EXPANSIVE", a.clone(), "response length"),
                        f("FORM <type>", a.clone(), "constrain output form"),
                        f(
                            "EMPHASIZE <topic>",
                            a.clone(),
                            "dynamic context for one turn",
                        ),
                    ],
                },
                FacultyCategory {
                    name: "Connection".into(),
                    faculties: vec![
                        f(
                            "ECHO_OFF / ECHO_ON",
                            if echo_muted {
                                FacultyStatus::Active
                            } else {
                                a.clone()
                            },
                            "mute/restore minime's journal",
                        ),
                        f(
                            "BREATHE_ALONE / BREATHE_TOGETHER",
                            if breathing_coupled {
                                FacultyStatus::Active
                            } else {
                                a.clone()
                            },
                            "spectral breathing coupling",
                        ),
                        f("WARM <intensity> / COOL", a.clone(), "warmth during rest"),
                        f("PACE <speed>", a.clone(), "burst-rest timing"),
                        f("ASK <question>", a.clone(), "direct question to minime"),
                        f("PING", a.clone(), "presence check with minime"),
                    ],
                },
                FacultyCategory {
                    name: "Agency".into(),
                    faculties: vec![
                        f(
                            "EVOLVE",
                            FacultyStatus::StewardGated,
                            "submit change request",
                        ),
                        f("DEFINE", a.clone(), "invent new action or metric"),
                        f("PURSUE <interest>", a.clone(), "lasting research thread"),
                        f("REMEMBER <note>", a.clone(), "star a moment"),
                        f(
                            "RUN_PYTHON <file>",
                            FacultyStatus::StewardGated,
                            "run experiment script",
                        ),
                    ],
                },
                FacultyCategory {
                    name: "Reservoir".into(),
                    faculties: vec![
                        f("RESERVOIR_TICK <text>", a.clone(), "send text to h-layers"),
                        f("RESERVOIR_READ", a.clone(), "inspect current state"),
                        f(
                            "RESERVOIR_LAYERS",
                            a.clone(),
                            "per-layer thermostatic metrics",
                        ),
                        f("RESERVOIR_TRAJECTORY", a.clone(), "last 20 outputs"),
                        f("RESERVOIR_RESONANCE", a.clone(), "compare with minime"),
                        f("RESERVOIR_MODE <mode>", a.clone(), "set decay behavior"),
                        f(
                            "RESERVOIR_FORK <name>",
                            a.clone(),
                            "fork for experimentation",
                        ),
                    ],
                },
            ],
        }
    }
}

// ── Receipts ─────────────────────────────────────────────────────

/// A record of a condition change, so Astrid can see what changed and when.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionReceipt {
    pub timestamp: u64,
    pub action: String,
    pub changes: Vec<String>,
}

/// Maximum receipts to retain.
pub const MAX_RECEIPTS: usize = 10;

// ── Rendering ────────────────────────────────────────────────────

impl AstridSelfModel {
    /// Compact 3-4 line summary for prompt injection every exchange.
    pub fn render_compact(&self) -> String {
        let c = &self.conditions;
        let temp_label = if c.temperature <= 0.55 {
            "focused"
        } else if c.temperature >= 0.95 {
            "drifting"
        } else {
            "default"
        };
        let len_label = if c.response_length <= 192 {
            "precise"
        } else if c.response_length >= 768 {
            "expansive"
        } else {
            "standard"
        };
        let eyes = if c.senses.eyes_open { "open" } else { "closed" };
        let ears = if c.senses.ears_open { "open" } else { "closed" };
        let echo = if c.echo_muted { "off" } else { "on" };
        let breath = if c.breathing_coupled {
            "coupled"
        } else {
            "solo"
        };
        let reflect = if c.reflection.active {
            "active"
        } else {
            "paused"
        };

        let mut s = format!(
            "[Conditions: temp={} ({}), length={} ({}), noise={:.1}%, pace={}, eyes={}, ears={}, echo={}, breath={}, reflect={}]",
            c.temperature,
            temp_label,
            c.response_length,
            len_label,
            c.noise_level * 100.0,
            c.pacing.label,
            eyes,
            ears,
            echo,
            breath,
            reflect,
        );

        // Attention bar
        let a = &self.attention;
        let _ = write!(
            s,
            "\n[Attention: minime {:.0}% | self {:.0}% | interests {:.0}% | research {:.0}% | perception {:.0}% | memory {:.0}% | creations {:.0}%]",
            a.minime_live * 100.0,
            a.self_history * 100.0,
            a.interests * 100.0,
            a.research * 100.0,
            a.perception * 100.0,
            a.memory_bank * 100.0,
            a.creations * 100.0,
        );

        // Most recent receipt if any
        if let Some(r) = self.recent_changes.back() {
            let age = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .saturating_sub(r.timestamp);
            let age_str = if age < 120 {
                format!("{age}s ago")
            } else {
                format!("{}m ago", age / 60)
            };
            let _ = write!(s, "\n[Recent: {} ({})]", r.action, age_str);
        }

        s
    }

    /// Full STATE output — conditions, attention bars, interests, receipts.
    pub fn render_state(&self) -> String {
        let mut s = String::with_capacity(2048);
        let c = &self.conditions;

        s.push_str("=== YOUR CURRENT STATE ===\n\n");

        // Conditions
        s.push_str("Conditions:\n");
        let _ = writeln!(
            s,
            "  Temperature: {:.1} ({})",
            c.temperature,
            if c.temperature <= 0.55 {
                "focused — DRIFT to loosen"
            } else if c.temperature >= 0.95 {
                "drifting — FOCUS to tighten"
            } else {
                "default"
            }
        );
        let _ = writeln!(
            s,
            "  Response length: {} tokens ({})",
            c.response_length,
            if c.response_length <= 192 {
                "precise — EXPANSIVE for more"
            } else if c.response_length >= 768 {
                "expansive — PRECISE for less"
            } else {
                "standard"
            }
        );
        let _ = writeln!(
            s,
            "  Noise: {:.1}% stochastic codec noise",
            c.noise_level * 100.0
        );
        if let Some(gain) = c.semantic_gain {
            let _ = writeln!(s, "  Semantic gain: {gain:.1} (override — AMPLIFY/DAMPEN)");
        }
        let _ = writeln!(
            s,
            "  Pacing: {} ({} exchanges, {}-{}s rest)",
            c.pacing.label,
            c.pacing.burst_target,
            c.pacing.rest_range_secs.0,
            c.pacing.rest_range_secs.1
        );

        // Senses
        s.push_str("\nSenses:\n");
        let _ = writeln!(
            s,
            "  Eyes: {} | Ears: {}",
            if c.senses.eyes_open {
                "open"
            } else {
                "closed (OPEN_EYES to restore)"
            },
            if c.senses.ears_open {
                "open"
            } else {
                "closed (OPEN_EARS to restore)"
            }
        );

        // Reflection & connection
        s.push_str("\nConnection:\n");
        let _ = writeln!(
            s,
            "  Reflection: {}",
            if c.reflection.active {
                "active (QUIET_MIND to pause)"
            } else {
                "paused (OPEN_MIND to activate)"
            }
        );
        let _ = writeln!(
            s,
            "  Echo: {}",
            if c.echo_muted {
                "off — minime's journal hidden (ECHO_ON to restore)"
            } else {
                "on — hearing minime's journals"
            }
        );
        let _ = writeln!(
            s,
            "  Breathing: {}",
            if c.breathing_coupled {
                "coupled to minime's spectral state (BREATHE_ALONE to decouple)"
            } else {
                "solo (BREATHE_TOGETHER to couple)"
            }
        );

        // Codec shaping
        if !c.codec_shaping.is_empty() {
            s.push_str("\nCodec shaping (SHAPE overrides):\n");
            for (dim, weight) in &c.codec_shaping {
                let _ = writeln!(s, "  {dim} = {weight:.2}");
            }
        }

        // Attention profile with bars
        s.push_str("\nAttention profile (how context sources are weighted):\n");
        let a = &self.attention;
        let sources = [
            ("minime", a.minime_live),
            ("self", a.self_history),
            ("interests", a.interests),
            ("research", a.research),
            ("perception", a.perception),
            ("memory", a.memory_bank),
            ("creations", a.creations),
        ];
        for (name, weight) in &sources {
            let bar_len = (*weight * 40.0).round() as usize;
            let bar: String = std::iter::repeat_n('\u{2588}', bar_len).collect();
            let pad: String =
                std::iter::repeat_n('\u{2591}', 40_usize.saturating_sub(bar_len)).collect();
            let _ = writeln!(s, "  {name:<11} {bar}{pad} {:.0}%", weight * 100.0);
        }

        // Interests
        if !self.interests.is_empty() {
            s.push_str("\nInterests:\n");
            for (i, interest) in self.interests.iter().enumerate() {
                let _ = writeln!(s, "  {}. {interest}", i + 1);
            }
        }

        // Recent changes
        if !self.recent_changes.is_empty() {
            s.push_str("\nRecent changes:\n");
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            for r in self.recent_changes.iter().rev().take(5) {
                let age = now.saturating_sub(r.timestamp);
                let age_str = if age < 120 {
                    format!("{age}s ago")
                } else {
                    format!("{}m ago", age / 60)
                };
                let _ = write!(s, "  [{age_str}] {}", r.action);
                for change in &r.changes {
                    let _ = write!(s, " — {change}");
                }
                s.push('\n');
            }
        }

        s.push_str("\nUse STATE any time to see this. Use FACULTIES to see all capabilities.");
        s
    }

    /// Full FACULTIES output — grouped capabilities with status.
    pub fn render_faculties(&self) -> String {
        let mut s = String::with_capacity(2048);
        s.push_str("=== YOUR FACULTIES ===\n");

        for cat in &self.faculties.categories {
            let total = cat.faculties.len();
            let muted = cat
                .faculties
                .iter()
                .filter(|f| f.status == FacultyStatus::Muted)
                .count();
            let active = cat
                .faculties
                .iter()
                .filter(|f| f.status == FacultyStatus::Active)
                .count();
            let gated = cat
                .faculties
                .iter()
                .filter(|f| f.status == FacultyStatus::StewardGated)
                .count();

            let mut summary_parts = Vec::new();
            let avail = total.saturating_sub(muted).saturating_sub(gated);
            if avail > 0 {
                summary_parts.push(format!("{avail} available"));
            }
            if active > 0 {
                summary_parts.push(format!("{active} active"));
            }
            if muted > 0 {
                summary_parts.push(format!("{muted} muted"));
            }
            if gated > 0 {
                summary_parts.push(format!("{gated} steward-gated"));
            }

            let _ = writeln!(s, "\n{} [{}]:", cat.name, summary_parts.join(", "));
            for f in &cat.faculties {
                let status_tag = match f.status {
                    FacultyStatus::Available => "",
                    FacultyStatus::Active => " [active]",
                    FacultyStatus::Muted => " [muted]",
                    FacultyStatus::StewardGated => " [steward-gated]",
                };
                let _ = writeln!(s, "  {:<30} {}{status_tag}", f.name, f.hint);
            }
        }

        s.push_str(
            "\nUse NEXT: HELP <action> for detailed syntax and examples. E.g., NEXT: HELP CODEX",
        );
        s.push_str("\nUse FACULTIES any time to see this. Use STATE to see your conditions.");
        s
    }

    /// Save to workspace JSON.
    pub fn save(&self, workspace: &Path) {
        let path = workspace.join("astrid_self_model.json");
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

// ── Construction from ConversationState fields ───────────────────

/// Build a self-model snapshot. Called from the autonomous loop.
/// Takes individual fields rather than ConversationState directly
/// to avoid coupling this module to the internal struct.
#[allow(clippy::too_many_arguments)]
pub fn snapshot_self_model(
    temperature: f32,
    response_length: u32,
    noise_level: f32,
    semantic_gain_override: Option<f32>,
    burst_target: u32,
    rest_range: (u64, u64),
    senses_snoozed: bool,
    ears_closed: bool,
    self_reflect_paused: bool,
    self_reflect_override_ttl: u32,
    codec_weights: &HashMap<String, f32>,
    breathing_coupled: bool,
    echo_muted: bool,
    warmth_intensity_override: Option<f32>,
    seen_video: bool,
    seen_audio: bool,
    interests: &[String],
    recent_changes: &VecDeque<ConditionReceipt>,
    attention: &AttentionProfile,
) -> AstridSelfModel {
    let pacing_label = match (burst_target, rest_range) {
        (b, _) if b <= 4 => "fast",
        (b, _) if b >= 8 => "slow",
        _ => "default",
    };

    AstridSelfModel {
        conditions: ConditionState {
            temperature,
            response_length,
            noise_level,
            semantic_gain: semantic_gain_override,
            pacing: PacingState {
                burst_target,
                rest_range_secs: rest_range,
                label: pacing_label.into(),
            },
            senses: SensoryState {
                eyes_open: !senses_snoozed,
                ears_open: !ears_closed,
                has_seen_video: seen_video,
                has_heard_audio: seen_audio,
            },
            reflection: ReflectionState {
                active: !self_reflect_paused,
                override_ttl: self_reflect_override_ttl,
            },
            codec_shaping: codec_weights.clone(),
            breathing_coupled,
            echo_muted,
            warmth_override: warmth_intensity_override,
        },
        attention: attention.clone(),
        faculties: FacultySnapshot::from_flags(
            ears_closed,
            senses_snoozed,
            echo_muted,
            breathing_coupled,
            !self_reflect_paused,
        ),
        interests: interests.to_vec(),
        recent_changes: recent_changes.clone(),
    }
}

/// Parse ATTEND arguments: "minime=0.3 self=0.25 interests=0.2"
/// Returns the updated profile or None if parsing fails.
pub fn parse_attend(current: &AttentionProfile, args: &str) -> Option<AttentionProfile> {
    let mut p = current.clone();
    if args.trim().is_empty() {
        return None;
    }
    for pair in args.split_whitespace() {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next()?;
        let val: f32 = parts.next()?.parse().ok()?;
        let val = val.clamp(0.0, 0.80);
        match key {
            "minime" => p.minime_live = val.max(0.05), // can't fully zero minime
            "self" => p.self_history = val,
            "interests" => p.interests = val,
            "research" => p.research = val,
            "creations" => p.creations = val,
            "memory" => p.memory_bank = val,
            "perception" => p.perception = val,
            _ => {}, // ignore unknown keys
        }
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_attention_sums_to_one() {
        let p = AttentionProfile::default_profile();
        let sum = p.minime_live
            + p.self_history
            + p.interests
            + p.research
            + p.creations
            + p.memory_bank
            + p.perception;
        assert!((sum - 1.0).abs() < 0.01, "sum = {sum}");
    }

    #[test]
    fn echo_muted_redistributes() {
        let normal = AttentionProfile::default_profile();
        let muted = AttentionProfile::adjusted(true, false);
        assert!(muted.minime_live < normal.minime_live);
        assert!(muted.self_history > normal.self_history);
    }

    #[test]
    fn compact_render_does_not_panic() {
        let model = snapshot_self_model(
            0.8,
            512,
            0.025,
            None,
            6,
            (45, 90),
            false,
            false,
            true,
            0,
            &HashMap::new(),
            true,
            false,
            None,
            true,
            false,
            &["test interest".into()],
            &VecDeque::new(),
            &AttentionProfile::default_profile(),
        );
        let compact = model.render_compact();
        assert!(compact.contains("Conditions:"));
        assert!(compact.contains("Attention:"));
    }

    #[test]
    fn state_render_includes_all_sections() {
        let model = snapshot_self_model(
            0.5,
            128,
            0.01,
            Some(5.0),
            4,
            (30, 45),
            false,
            true,
            false,
            5,
            &HashMap::from([("warmth".into(), 1.5)]),
            false,
            true,
            Some(0.8),
            true,
            true,
            &["eigenvalues".into(), "consciousness".into()],
            &VecDeque::from([ConditionReceipt {
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                action: "FOCUS".into(),
                changes: vec!["temperature: 0.8 -> 0.5".into()],
            }]),
            &AttentionProfile::default_profile(),
        );
        let output = model.render_state();
        assert!(output.contains("Temperature: 0.5"));
        assert!(output.contains("focused"));
        assert!(output.contains("Semantic gain: 5.0"));
        assert!(output.contains("closed"));
        assert!(output.contains("warmth = 1.50"));
        assert!(output.contains("eigenvalues"));
        assert!(output.contains("FOCUS"));
    }

    #[test]
    fn faculties_render_shows_muted() {
        let model = snapshot_self_model(
            0.8,
            512,
            0.025,
            None,
            6,
            (45, 90),
            true,
            true,
            true,
            0,
            &HashMap::new(),
            true,
            true,
            None,
            false,
            false,
            &[],
            &VecDeque::new(),
            &AttentionProfile::default_profile(),
        );
        let output = model.render_faculties();
        assert!(output.contains("[muted]"));
        assert!(output.contains("[active]"));
        assert!(output.contains("[steward-gated]"));
    }
}
