use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::hebbian::HebbianCodecSidecar;
use crate::journal::{RemoteJournalEntry, scan_remote_journal_dir};
use crate::memory::RemoteMemorySummary;
use crate::types::SafetyLevel;

const RESEARCH_NEW_GROUND_LOOKBACK_EXCHANGES: u64 = 6;
const MAX_NEW_GROUND_BUDGET: u8 = 3;
const MAX_RECENT_RESEARCH_PROGRESS: usize = 16;
const READ_DEPTH_ADVANCE_MIN_CHARS: u32 = 1_000;
const MAX_PENDING_HEBBIAN_OUTCOMES: usize = 4;

/// Snapshot of spectral + reservoir state at PERTURB time.
/// Consumed on the next exchange to show Astrid the temporal ripple.
#[derive(Debug, Clone)]
pub(crate) struct PerturbBaseline {
    pub fill_pct: f32,
    pub lambda1: f32,
    pub eigenvalues: Vec<f32>,
    pub description: String,
    pub timestamp: std::time::Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct PendingHebbianOutcome {
    pub exchange_count: u64,
    pub signature: Vec<f32>,
    pub fill_before: f32,
    pub telemetry_t_ms_before: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct ResearchProgressReceipt {
    #[serde(default)]
    pub exchange_count: u64,
    #[serde(default)]
    pub issued_at_unix_s: u64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    #[serde(alias = "action")]
    pub source_action: String,
    #[serde(default)]
    pub cluster_key: String,
    #[serde(default)]
    #[serde(alias = "topic")]
    pub topic_key: Option<String>,
    #[serde(default)]
    #[serde(alias = "theme")]
    pub theme_key: Option<String>,
    #[serde(default)]
    pub subject_key: Option<String>,
    #[serde(default)]
    pub related_key: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub delta_chars: Option<u32>,
    #[serde(default)]
    pub artifact_path: Option<String>,
    #[serde(default)]
    pub dedupe_key: String,
    #[serde(default)]
    pub credit: u8,
    #[serde(default)]
    pub ttl_exchanges: u8,
}

impl ResearchProgressReceipt {
    fn repair_defaults(&mut self) {
        self.source_action = normalize_choice_action(&self.source_action);
        self.kind = self.kind.trim().to_string();
        self.label = clean_optional_text(self.label.take());
        self.path = clean_optional_text(self.path.take());
        self.url = clean_optional_text(self.url.take());
        self.topic_key = normalize_optional_receipt_key(self.topic_key.as_deref());
        self.theme_key = normalize_optional_receipt_key(self.theme_key.as_deref()).or_else(|| {
            self.topic_key
                .as_deref()
                .and_then(|topic| dominant_focus_theme(&tokenize_focus_text(topic)))
        });
        self.subject_key = self
            .subject_key
            .as_deref()
            .and_then(normalize_receipt_key)
            .or_else(|| self.label.as_deref().and_then(normalize_receipt_key))
            .or_else(|| self.url.as_deref().and_then(normalize_receipt_key))
            .or_else(|| self.path.as_deref().and_then(normalize_receipt_key));
        self.related_key = self.related_key.as_deref().and_then(normalize_receipt_key);
        if self.credit == 0 || self.ttl_exchanges == 0 {
            let (credit, ttl_exchanges) = receipt_kind_defaults(&self.kind);
            if self.credit == 0 {
                self.credit = credit;
            }
            if self.ttl_exchanges == 0 {
                self.ttl_exchanges = ttl_exchanges;
            }
        }
        if self.cluster_key.trim().is_empty() {
            self.cluster_key = derive_cluster_key(
                self.topic_key.as_deref(),
                self.theme_key.as_deref(),
                self.subject_key.as_deref(),
                &self.source_action,
            );
        }
        if self.dedupe_key.trim().is_empty() {
            self.dedupe_key = derive_dedupe_key(
                &self.kind,
                &self.cluster_key,
                self.subject_key.as_deref(),
                self.related_key.as_deref(),
            );
        }
    }

    fn is_active_at(&self, current_exchange: u64) -> bool {
        let ttl = u64::from(self.ttl_exchanges);
        ttl > 0 && current_exchange.saturating_sub(self.exchange_count) < ttl
    }

    fn matches_inquiry(
        &self,
        cluster_key: &str,
        topic_key: Option<&str>,
        theme_key: Option<&str>,
        subject_key: Option<&str>,
    ) -> bool {
        (!cluster_key.is_empty() && self.cluster_key == cluster_key)
            || topic_key.is_some_and(|topic| self.topic_key.as_deref() == Some(topic))
            || theme_key.is_some_and(|theme| self.theme_key.as_deref() == Some(theme))
            || subject_key.is_some_and(|subject| self.subject_key.as_deref() == Some(subject))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct NextChoiceFeedback {
    pub hint: Option<String>,
    pub override_action: Option<String>,
    pub progress_sensitive: bool,
    pub stagnant_loop: bool,
    pub new_ground_budget: u8,
}

impl NextChoiceFeedback {
    fn hinted(hint: String) -> Self {
        Self {
            hint: Some(hint),
            override_action: None,
            progress_sensitive: false,
            stagnant_loop: false,
            new_ground_budget: 0,
        }
    }

    fn progress_hinted(hint: String, new_ground_budget: u8) -> Self {
        Self {
            hint: Some(hint),
            override_action: None,
            progress_sensitive: true,
            stagnant_loop: false,
            new_ground_budget,
        }
    }

    fn stagnant_forced(
        hint: String,
        override_action: impl Into<String>,
        new_ground_budget: u8,
    ) -> Self {
        Self {
            hint: Some(hint),
            override_action: Some(override_action.into()),
            progress_sensitive: false,
            stagnant_loop: true,
            new_ground_budget,
        }
    }
}

fn is_analysis_action(base: &str) -> bool {
    matches!(base, "EXAMINE" | "EXAMINE_CODE" | "INTROSPECT")
}

fn is_research_like_action(base: &str) -> bool {
    is_analysis_action(base) || matches!(base, "BROWSE" | "SEARCH" | "READ_MORE")
}

fn normalize_choice_action(action: &str) -> String {
    action
        .split_whitespace()
        .next()
        .unwrap_or(action)
        .trim()
        .to_uppercase()
}

fn clean_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn normalize_receipt_key(value: &str) -> Option<String> {
    let trimmed = value
        .trim()
        .trim_matches(|c: char| matches!(c, '[' | ']' | '"' | '\'' | '`' | '“' | '”'))
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains("://") || trimmed.contains('/') || trimmed.contains('.') {
        return Some(trimmed.to_ascii_lowercase());
    }
    let tokens = tokenize_focus_text(trimmed);
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

fn normalize_optional_receipt_key(value: Option<&str>) -> Option<String> {
    value.and_then(normalize_receipt_key)
}

fn derive_cluster_key(
    topic_key: Option<&str>,
    theme_key: Option<&str>,
    subject_key: Option<&str>,
    source_action: &str,
) -> String {
    topic_key
        .map(ToOwned::to_owned)
        .or_else(|| theme_key.map(ToOwned::to_owned))
        .or_else(|| subject_key.map(ToOwned::to_owned))
        .unwrap_or_else(|| source_action.to_string())
}

fn derive_dedupe_key(
    kind: &str,
    cluster_key: &str,
    subject_key: Option<&str>,
    related_key: Option<&str>,
) -> String {
    format!(
        "{kind}|{cluster_key}|{}|{}",
        subject_key.unwrap_or_default(),
        related_key.unwrap_or_default()
    )
}

fn receipt_kind_defaults(kind: &str) -> (u8, u8) {
    match kind {
        "new_source_resolved" | "new_page_context" | "cross_link_formed" => (2, 4),
        "read_depth_advance" => (1, 1),
        _ => (0, 0),
    }
}

fn extract_choice_subject(choice: &str) -> Option<String> {
    let trimmed = choice.trim();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let base = parts.next()?.trim().to_uppercase();
    if !is_research_like_action(base.as_str()) {
        return None;
    }
    let rest = parts.next().unwrap_or("").trim();
    if rest.is_empty() {
        return None;
    }
    let canonical = rest
        .split_once('=')
        .map(|(_, value)| value)
        .unwrap_or(rest)
        .trim()
        .trim_matches(|c: char| matches!(c, '[' | ']' | '"' | '\'' | '`' | '“' | '”'))
        .trim();
    normalize_receipt_key(canonical)
}

fn tokenize_focus_text(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn classify_focus_theme_token(token: &str) -> Option<&'static str> {
    if token.starts_with("eigen")
        || token.starts_with("lambda")
        || matches!(
            token,
            "variance"
                | "ratio"
                | "ratios"
                | "gap"
                | "gaps"
                | "pair"
                | "pairs"
                | "mode"
                | "modes"
                | "dominant"
                | "spectral"
                | "entropy"
                | "cascade"
                | "fold"
                | "folding"
                | "folded"
                | "selection"
                | "pruning"
                | "prune"
                | "collapse"
                | "collapsed"
                | "compression"
                | "compress"
                | "compressed"
                | "channel"
                | "channeled"
                | "sculpt"
                | "sculpting"
                | "shape"
                | "shaping"
                | "shadowing"
                | "thinning"
                | "bottleneck"
        )
    {
        return Some("spectral folding / selection");
    }
    if matches!(
        token,
        "surge"
            | "threshold"
            | "stale"
            | "semantic"
            | "decay"
            | "linger"
            | "controller"
            | "control"
            | "regulator"
            | "regulation"
            | "integral"
            | "feedback"
            | "gain"
            | "gate"
            | "filter"
            | "damping"
            | "step"
            | "target"
    ) || token.starts_with("regulat")
    {
        return Some("control / regulation");
    }
    if matches!(
        token,
        "shadow" | "ising" | "magnetization" | "field" | "attractor"
    ) {
        return Some("shadow-field / attractor");
    }
    if matches!(
        token,
        "memory"
            | "memories"
            | "glimpse"
            | "glimpses"
            | "resonance"
            | "resonant"
            | "codec"
            | "embedding"
    ) {
        return Some("memory / resonance");
    }
    None
}

fn dominant_focus_theme(tokens: &[String]) -> Option<String> {
    let mut counts = HashMap::<&'static str, usize>::new();
    for token in tokens {
        if let Some(theme) = classify_focus_theme_token(token) {
            *counts.entry(theme).or_insert(0) += 1;
        }
    }
    if counts.is_empty() {
        return None;
    }

    let priorities = [
        "spectral folding / selection",
        "control / regulation",
        "shadow-field / attractor",
        "memory / resonance",
    ];
    let mut best: Option<(&'static str, usize)> = None;
    for theme in priorities {
        let count = counts.get(theme).copied().unwrap_or(0);
        if count == 0 {
            continue;
        }
        match best {
            Some((_, best_count)) if best_count >= count => {},
            _ => best = Some((theme, count)),
        }
    }
    best.map(|(theme, _)| theme.to_string())
}

fn normalize_focus_topic(choice: &str) -> Option<String> {
    let trimmed = choice.trim();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let base = parts.next()?.trim().to_uppercase();
    if !is_analysis_action(base.as_str()) {
        return None;
    }

    let rest = parts.next().unwrap_or("").trim();
    if rest.is_empty() {
        return None;
    }

    let canonical = rest
        .split_once('=')
        .map(|(_, value)| value)
        .unwrap_or(rest)
        .trim()
        .trim_matches(|c: char| matches!(c, '[' | ']' | '"' | '\'' | '`' | '“' | '”'))
        .trim();
    if canonical.is_empty() {
        return None;
    }

    let raw_tokens = tokenize_focus_text(canonical);
    let tokens: Vec<String> = raw_tokens
        .iter()
        .filter(|part| {
            !matches!(
                part.as_str(),
                "target" | "file" | "path" | "code" | "rs" | "py" | "md" | "json" | "toml" | "txt"
            )
        })
        .cloned()
        .collect();
    let normalized = if tokens.is_empty() {
        raw_tokens
    } else {
        tokens
    };
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.join(" "))
    }
}

fn normalize_focus_theme(choice: &str) -> Option<String> {
    let trimmed = choice.trim();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let base = parts.next()?.trim().to_uppercase();
    if !is_analysis_action(base.as_str()) {
        return None;
    }

    let rest = parts.next().unwrap_or("").trim();
    if rest.is_empty() {
        return None;
    }

    let canonical = rest
        .split_once('=')
        .map(|(_, value)| value)
        .unwrap_or(rest)
        .trim()
        .trim_matches(|c: char| matches!(c, '[' | ']' | '"' | '\'' | '`' | '“' | '”'))
        .trim();
    if canonical.is_empty() {
        return None;
    }

    dominant_focus_theme(&tokenize_focus_text(canonical))
}

/// Conversational mode for each exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    /// Feed minime's own journal text back as sensory input.
    Mirror,
    /// Astrid's philosophical response engaging with minime's themes.
    Dialogue,
    /// Astrid witnesses and describes the spectral state poetically.
    Witness,
    /// Astrid reads its own or minime's source code and reflects.
    Introspect,
    /// Astrid turns longing into a governed agency request.
    Evolve,
    /// Astrid proposes a spectral experiment and observes the result.
    Experiment,
    /// Unstructured thought during rest — Astrid's own daydream, not a response.
    Daydream,
    /// Growth reflection — what Astrid wants to become, experience, or change.
    Aspiration,
    /// Event-driven — a spectral phase transition just happened; capture the moment.
    MomentCapture,
    /// Original creative work — not a response, a creation.
    Create,
    /// Self-initiated — Astrid generates her own prompt from her own context.
    Initiate,
    /// Contemplative presence — no generation, no NEXT: choice.
    Contemplate,
}

/// A timestamped spectral snapshot for tracking rates of change.
#[derive(Clone)]
pub(super) struct SpectralSample {
    pub fill: f32,
    pub lambda1: f32,
    pub ts: std::time::Instant,
}

/// Tracks conversational context across iterations.
pub(super) struct ConversationState {
    pub prev_fill: f32,
    /// Ring buffer of recent (fill, lambda1, timestamp) samples for rate-of-change
    /// and multi-horizon trend reporting. Capped at 30 entries (~10 minutes of exchanges).
    pub spectral_history: VecDeque<SpectralSample>,
    pub exchange_count: u64,
    pub last_mode: Mode,
    /// Cached remote minime journal entries (newest first, periodically rescanned).
    /// This is intentionally distinct from Astrid's own journal directory.
    pub remote_journal_entries: Vec<RemoteJournalEntry>,
    /// Number of remote journal entries at last scan (to detect new entries).
    pub remote_journal_count_at_scan: usize,
    /// Index into the dialogue pool (rotates).
    pub dialogue_cursor: usize,
    /// Remote minime workspace path for rescanning.
    pub remote_workspace: Option<PathBuf>,
    /// New minime self-study waiting for an immediate Astrid response.
    pub pending_remote_self_study: Option<RemoteJournalEntry>,
    /// Recent conversation history for statefulness (last N exchanges).
    pub history: Vec<crate::llm::Exchange>,
    /// Index into the introspection source file list.
    pub introspect_cursor: usize,
    pub seen_video: bool,
    pub seen_audio: bool,
    /// Astrid chose NEXT: LOOK — include ANSI spatial art in next exchange.
    pub wants_look: bool,
    /// Astrid chose NEXT: SEARCH — force web search enrichment on next exchange.
    pub wants_search: bool,
    /// Self-referential loop: dynamic by default, Astrid can override with
    /// QUIET_MIND / OPEN_MIND.
    pub self_reflect_paused: bool,
    /// Explicit override from QUIET_MIND / OPEN_MIND — cleared after N exchanges.
    pub self_reflect_override: Option<bool>,
    /// Countdown: exchanges remaining before the override expires.
    pub self_reflect_override_ttl: u32,
    /// Independent audio control — CLOSE_EARS / OPEN_EARS
    pub ears_closed: bool,
    /// Astrid chose a form constraint (NEXT: FORM poem, FORM equation, etc.)
    pub form_constraint: Option<String>,
    /// Astrid specified a search topic (NEXT: SEARCH "topic goes here").
    pub search_topic: Option<String>,
    /// Astrid chose NEXT: BROWSE <url> — fetch and read a full web page.
    pub browse_url: Option<String>,
    /// Most recent research thread anchor — used to interpret follow-up browsing.
    pub last_research_anchor: Option<String>,
    /// Path to the last browsed/read file, for READ_MORE continuation.
    pub last_read_path: Option<String>,
    /// Character offset into last_read_path for READ_MORE.
    pub last_read_offset: usize,
    /// Meaning summary for the last browsed document — reused by READ_MORE.
    pub last_read_meaning_summary: Option<String>,
    /// Astrid chose NEXT: INTROSPECT — force introspection mode next exchange.
    pub wants_introspect: bool,
    /// Optional: specific source label and line offset for targeted introspection.
    pub introspect_target: Option<(String, usize)>,
    /// Astrid chose NEXT: REVISE [keyword] — load a previous creation and iterate.
    pub revise_keyword: Option<String>,
    /// Astrid chose NEXT: COMPOSE or VOICE — generate WAV from spectral state.
    pub wants_compose_audio: bool,
    /// Astrid chose NEXT: ANALYZE_AUDIO — analyze inbox WAV.
    pub wants_analyze_audio: bool,
    /// Astrid chose NEXT: RENDER_AUDIO [mode] — run inbox WAV through chimera.
    pub wants_render_audio: Option<String>,
    /// Astrid chose NEXT: EVOLVE — turn longing into a request on next exchange.
    pub wants_evolve: bool,
    /// Astrid explicitly chose a mode for next exchange (DAYDREAM, ASPIRE).
    pub next_mode_override: Option<Mode>,
    /// Astrid chose NEXT: DECOMPOSE — full spectral analysis next exchange.
    pub wants_decompose: bool,
    /// Previous eigenvalues for per-mode velocity computation in DECOMPOSE.
    pub prev_eigenvalues: Option<Vec<f32>>,
    /// Astrid chose NEXT: THINK_DEEP — use reasoning model next exchange.
    pub wants_deep_think: bool,
    /// Astrid chose NEXT: EXAMINE — force all viz blocks on next exchange.
    pub force_all_viz: bool,
    /// Spectral snapshot from Astrid's last PERTURB — consumed next exchange
    /// to show her the before/after delta (temporal feedback).
    pub perturb_baseline: Option<PerturbBaseline>,
    /// Astrid (or minime) chose to snooze sensory input — suppress perceptions.
    pub senses_snoozed: bool,
    // Astrid's stylistic sovereignty
    pub creative_temperature: f32,
    pub response_length: u32,
    pub emphasis: Option<String>,
    /// Previous RASCII 8D visual features for change tracking.
    pub last_visual_features: Option<Vec<f32>>,
    /// Ring buffer of last 5 NEXT: choices — used to detect fixation patterns.
    pub recent_next_choices: VecDeque<String>,
    /// Ring buffer of repeated semantic targets (EXAMINE / INTROSPECT topics).
    pub recent_focus_topics: VecDeque<String>,
    /// Ring buffer of broader analysis themes spanning multiple exact topics.
    pub recent_focus_themes: VecDeque<String>,
    /// Ring buffer of last 8 BROWSE URLs — used to detect URL attractor patterns.
    pub recent_browse_urls: VecDeque<String>,
    /// Narrow new-ground receipts that prove a research loop is still advancing.
    pub recent_research_progress: VecDeque<ResearchProgressReceipt>,

    // --- Codec sovereignty (Phase A) ---
    /// Override semantic gain (default 2.0, action range 0.5-5.0).
    pub semantic_gain_override: Option<f32>,
    /// Override stochastic noise level (default 0.025 = 2.5%, range 0.005-0.05).
    pub noise_level: f32,
    /// Emotional dimension weights: "warmth" → dim 24 multiplier, etc.
    /// Explicit overrides from Astrid's SHAPE commands.
    pub codec_weights: HashMap<String, f32>,
    /// Data-driven weights from codec→fill correlation analysis.
    /// Merged with codec_weights at encoding time; SHAPE overrides win.
    pub learned_codec_weights: HashMap<String, f32>,
    /// Small pairwise co-activation sidecar that learns which dimension
    /// combinations tend to move fill toward a healthier center.
    pub hebbian_codec: HebbianCodecSidecar,
    /// One-shot contact receipts that wait for a newer Minime telemetry sample
    /// before teaching the Hebbian sidecar about the sent exchange.
    pub pending_hebbian_outcomes: VecDeque<PendingHebbianOutcome>,
    /// Telemetry watermark for queued Hebbian receipt consumption. Prevents
    /// multiple queued receipts from being consumed on the same telemetry tick.
    pub last_hebbian_consumed_telemetry_t_ms: Option<u64>,
    /// Warmth intensity override for rest phase (0.0-1.0, None = default taper).
    pub warmth_intensity_override: Option<f32>,
    /// Whether breathing is coupled to minime's spectral state.
    pub breathing_coupled: bool,
    /// Last GESTURE intention, persists as a "seed" in the warmth vector.
    pub last_gesture_seed: Option<Vec<f32>>,
    /// Burst-rest pacing: exchanges per burst.
    pub burst_target: u32,
    /// Burst-rest pacing: rest duration range (min_secs, max_secs).
    pub rest_range: (u64, u64),
    /// Astrid chose to mute minime's journal context.
    pub echo_muted: bool,
    /// Codec feedback: how Astrid's last response encoded into spectral features.
    pub last_codec_feedback: Option<String>,
    /// Previous exchange's raw codec features — used for delta encoding.
    pub last_codec_features: Option<Vec<f32>>,
    /// Cross-exchange codec signature — mean semantic shape over the last
    /// completed utterance, used for slower Hebbian updates.
    pub last_exchange_codec_signature: Option<Vec<f32>>,
    /// Sliding-window character frequency for cross-exchange entropy.
    pub char_freq_window: crate::codec::CharFreqWindow,
    /// Thematic resonance history — tracks recurring text types across exchanges.
    /// Strengthens codec gain when the same conversational direction is sustained.
    pub text_type_history: crate::codec::TextTypeHistory,
    /// Result of LIST_FILES — directory listing injected into next prompt.
    pub pending_file_listing: Option<String>,
    /// Lasting self-directed interests. Persist across restarts via state.json.
    pub interests: Vec<String>,
    /// Lightweight regime tracker — classifies spectral state every exchange.
    pub regime_tracker: crate::reflective::RegimeTracker,
    /// Astrid chose DEFER — acknowledge inbox without forced dialogue response.
    pub defer_inbox: bool,
    /// Selected remote 12D vague-memory glimpse from Minime.
    pub last_remote_glimpse_12d: Option<Vec<f32>>,
    /// Selected remote memory ID and role, mirrored from Minime.
    pub last_remote_memory_id: Option<String>,
    pub last_remote_memory_role: Option<String>,
    /// Compact summaries of Minime's available memory-bank entries.
    pub remote_memory_bank: Vec<RemoteMemorySummary>,
    /// Timestamp of last minime outbox scan — routes replies into Astrid's inbox.
    pub last_outbox_scan_ts: u64,
    /// Exchange count at which codec correlations were last recomputed.
    pub last_correlation_exchange: u64,
    /// Recent condition change receipts — visible in STATE and prompt block.
    pub condition_receipts: VecDeque<crate::self_model::ConditionReceipt>,
    /// Attention profile — how context sources are weighted in prompt assembly.
    /// Astrid can adjust via ATTEND. Drives actual source inclusion counts.
    pub attention: crate::self_model::AttentionProfile,
    /// One non-immediate thread sampled during rest — injected into next
    /// self-directed mode (Daydream, Aspiration, Initiate).
    pub peripheral_resonance: Option<String>,
    /// Last response from Codex relay — consumed by WRITE_FILE FROM_CODEX.
    pub last_codex_response: Option<String>,
    /// Thread ID for multi-turn Codex conversations.
    pub codex_thread_id: Option<String>,
}

impl ConversationState {
    pub(super) fn new(
        remote_journal_entries: Vec<RemoteJournalEntry>,
        remote_workspace: Option<PathBuf>,
    ) -> Self {
        let count = remote_journal_entries.len();
        Self {
            prev_fill: 0.0,
            spectral_history: VecDeque::with_capacity(30),
            exchange_count: 0,
            last_mode: Mode::Witness,
            remote_journal_entries,
            remote_journal_count_at_scan: count,
            dialogue_cursor: 0,
            remote_workspace,
            pending_remote_self_study: None,
            history: Vec::new(),
            introspect_cursor: 0,
            seen_video: false,
            seen_audio: false,
            wants_look: false,
            wants_search: false,
            senses_snoozed: false,
            self_reflect_paused: true,
            self_reflect_override: None,
            self_reflect_override_ttl: 0,
            ears_closed: false,
            form_constraint: None,
            search_topic: None,
            browse_url: None,
            last_research_anchor: None,
            last_read_path: None,
            last_read_offset: 0,
            last_read_meaning_summary: None,
            wants_introspect: false,
            introspect_target: None,
            revise_keyword: None,
            wants_compose_audio: false,
            wants_analyze_audio: false,
            wants_render_audio: None,
            wants_evolve: false,
            next_mode_override: None,
            wants_decompose: false,
            prev_eigenvalues: None,
            wants_deep_think: false,
            force_all_viz: false,
            perturb_baseline: None,
            creative_temperature: 0.8,
            response_length: 768,
            emphasis: None,
            last_visual_features: None,
            recent_next_choices: VecDeque::with_capacity(12),
            recent_focus_topics: VecDeque::with_capacity(8),
            recent_focus_themes: VecDeque::with_capacity(10),
            recent_browse_urls: VecDeque::with_capacity(8),
            recent_research_progress: VecDeque::with_capacity(MAX_RECENT_RESEARCH_PROGRESS),
            semantic_gain_override: None,
            noise_level: 0.005,
            codec_weights: HashMap::new(),
            learned_codec_weights: HashMap::new(),
            hebbian_codec: HebbianCodecSidecar::default(),
            pending_hebbian_outcomes: VecDeque::with_capacity(MAX_PENDING_HEBBIAN_OUTCOMES),
            last_hebbian_consumed_telemetry_t_ms: None,
            warmth_intensity_override: None,
            breathing_coupled: true,
            echo_muted: false,
            last_gesture_seed: None,
            burst_target: 6,
            rest_range: (45, 90),
            last_codec_feedback: None,
            last_codec_features: None,
            last_exchange_codec_signature: None,
            char_freq_window: crate::codec::CharFreqWindow::new(),
            text_type_history: crate::codec::TextTypeHistory::new(),
            pending_file_listing: None,
            interests: Vec::new(),
            last_remote_glimpse_12d: None,
            last_remote_memory_id: None,
            last_remote_memory_role: None,
            remote_memory_bank: Vec::new(),
            regime_tracker: crate::reflective::RegimeTracker::new(),
            defer_inbox: false,
            // Start scanning from recent — don't flood inbox with old backlog.
            last_outbox_scan_ts: 1_774_647_800,
            last_correlation_exchange: 0,
            condition_receipts: VecDeque::with_capacity(crate::self_model::MAX_RECEIPTS),
            attention: crate::self_model::AttentionProfile::default_profile(),
            peripheral_resonance: None,
            last_codex_response: None,
            codex_thread_id: None,
        }
    }

    /// Push a condition change receipt, capped at MAX_RECEIPTS.
    pub(super) fn push_receipt(&mut self, action: &str, changes: Vec<String>) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.condition_receipts
            .push_back(crate::self_model::ConditionReceipt {
                timestamp: ts,
                action: action.into(),
                changes,
            });
        while self.condition_receipts.len() > crate::self_model::MAX_RECEIPTS {
            self.condition_receipts.pop_front();
        }
    }

    pub(super) fn arm_pending_hebbian_outcome(
        &mut self,
        signature: Vec<f32>,
        fill_before: f32,
        telemetry_t_ms_before: Option<u64>,
    ) {
        if signature.is_empty() {
            return;
        }
        if self.pending_hebbian_outcomes.len() >= MAX_PENDING_HEBBIAN_OUTCOMES {
            let dropped = self.pending_hebbian_outcomes.pop_front();
            warn!(
                dropped_exchange = dropped.as_ref().map(|receipt| receipt.exchange_count),
                kept = MAX_PENDING_HEBBIAN_OUTCOMES.saturating_sub(1),
                "dropping oldest pending Hebbian outcome to preserve bounded FIFO"
            );
        }
        self.pending_hebbian_outcomes
            .push_back(PendingHebbianOutcome {
                exchange_count: self.exchange_count,
                signature,
                fill_before,
                telemetry_t_ms_before,
            });
    }

    pub(super) fn take_pending_hebbian_outcome_for_telemetry(
        &mut self,
        telemetry_t_ms: u64,
    ) -> Option<PendingHebbianOutcome> {
        if self
            .last_hebbian_consumed_telemetry_t_ms
            .is_some_and(|last| telemetry_t_ms <= last)
        {
            return None;
        }
        let should_consume = self
            .pending_hebbian_outcomes
            .front()
            .is_some_and(|receipt| {
                receipt
                    .telemetry_t_ms_before
                    .is_none_or(|before| telemetry_t_ms > before)
            });
        if !should_consume {
            return None;
        }
        self.last_hebbian_consumed_telemetry_t_ms = Some(telemetry_t_ms);
        self.pending_hebbian_outcomes.pop_front()
    }

    pub(super) fn repair_pending_hebbian_outcomes(&mut self) {
        while self.pending_hebbian_outcomes.len() > MAX_PENDING_HEBBIAN_OUTCOMES {
            self.pending_hebbian_outcomes.pop_front();
        }
    }

    fn push_focus_theme(&mut self, theme: String) {
        self.recent_focus_themes.push_back(theme);
        if self.recent_focus_themes.len() > 10 {
            self.recent_focus_themes.pop_front();
        }
    }

    pub(super) fn repair_research_progress_receipts(&mut self) {
        for receipt in &mut self.recent_research_progress {
            receipt.repair_defaults();
        }
    }

    fn record_new_ground_receipt(
        &mut self,
        source_action: &str,
        kind: &str,
        topic_key: Option<String>,
        theme_key: Option<String>,
        subject_key: Option<String>,
        related_key: Option<String>,
        label: Option<String>,
        path: Option<String>,
        url: Option<String>,
        delta_chars: Option<u32>,
    ) {
        let normalized_action = normalize_choice_action(source_action);
        let clean_label = clean_optional_text(label);
        let clean_path = clean_optional_text(path);
        let clean_url = clean_optional_text(url);
        let normalized_topic = normalize_optional_receipt_key(topic_key.as_deref());
        let normalized_theme = normalize_optional_receipt_key(theme_key.as_deref()).or_else(|| {
            normalized_topic
                .as_deref()
                .and_then(|topic| dominant_focus_theme(&tokenize_focus_text(topic)))
        });
        let normalized_subject = subject_key
            .as_deref()
            .and_then(normalize_receipt_key)
            .or_else(|| clean_label.as_deref().and_then(normalize_receipt_key))
            .or_else(|| clean_url.as_deref().and_then(normalize_receipt_key))
            .or_else(|| clean_path.as_deref().and_then(normalize_receipt_key));
        let normalized_related = related_key.as_deref().and_then(normalize_receipt_key);
        let cluster_key = derive_cluster_key(
            normalized_topic.as_deref(),
            normalized_theme.as_deref(),
            normalized_subject.as_deref(),
            &normalized_action,
        );
        let dedupe_key = derive_dedupe_key(
            kind,
            &cluster_key,
            normalized_subject.as_deref(),
            normalized_related.as_deref(),
        );
        let (credit, ttl_exchanges) = receipt_kind_defaults(kind);
        let issued_at_unix_s = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if let Some(existing) = self.recent_research_progress.iter_mut().find(|receipt| {
            receipt.dedupe_key == dedupe_key && receipt.is_active_at(self.exchange_count)
        }) {
            existing.exchange_count = self.exchange_count;
            existing.issued_at_unix_s = issued_at_unix_s;
            existing.kind = kind.to_string();
            existing.source_action = normalized_action.clone();
            existing.cluster_key = cluster_key.clone();
            existing.topic_key = normalized_topic.clone();
            existing.theme_key = normalized_theme.clone();
            existing.subject_key = normalized_subject.clone();
            existing.related_key = normalized_related.clone();
            existing.label = clean_label.clone();
            existing.path = clean_path.clone();
            existing.url = clean_url.clone();
            existing.delta_chars = delta_chars;
            existing.credit = credit;
            existing.ttl_exchanges = ttl_exchanges;
            return;
        }

        let receipt = ResearchProgressReceipt {
            exchange_count: self.exchange_count,
            issued_at_unix_s,
            kind: kind.to_string(),
            source_action: normalized_action.clone(),
            cluster_key: cluster_key.clone(),
            topic_key: normalized_topic.clone(),
            theme_key: normalized_theme.clone(),
            subject_key: normalized_subject.clone(),
            related_key: normalized_related.clone(),
            label: clean_label.clone(),
            path: clean_path.clone(),
            url: clean_url.clone(),
            delta_chars,
            artifact_path: None,
            dedupe_key: dedupe_key.clone(),
            credit,
            ttl_exchanges,
        };
        self.recent_research_progress.push_back(receipt);
        while self.recent_research_progress.len() > MAX_RECENT_RESEARCH_PROGRESS {
            self.recent_research_progress.pop_front();
        }
        info!(
            source_action = %normalized_action,
            kind = %kind,
            cluster_key = %cluster_key,
            topic_key = normalized_topic.as_deref().unwrap_or(""),
            theme_key = normalized_theme.as_deref().unwrap_or(""),
            subject_key = normalized_subject.as_deref().unwrap_or(""),
            related_key = normalized_related.as_deref().unwrap_or(""),
            label = clean_label.as_deref().unwrap_or(""),
            path = clean_path.as_deref().unwrap_or(""),
            url = clean_url.as_deref().unwrap_or(""),
            delta_chars = delta_chars.unwrap_or(0),
            credit,
            ttl_exchanges,
            "research progress receipt"
        );
    }

    fn recent_subject_seen(&self, kind: &str, subject_key: &str) -> bool {
        let earliest_exchange = self
            .exchange_count
            .saturating_sub(RESEARCH_NEW_GROUND_LOOKBACK_EXCHANGES);
        self.recent_research_progress.iter().any(|receipt| {
            receipt.exchange_count >= earliest_exchange
                && receipt.kind == kind
                && receipt.subject_key.as_deref() == Some(subject_key)
        })
    }

    pub(super) fn note_new_source_resolved(
        &mut self,
        source_action: &str,
        label: String,
        path: Option<String>,
        topic_key: Option<String>,
        theme_key: Option<String>,
    ) {
        let normalized_subject = normalize_receipt_key(&label)
            .or_else(|| path.as_deref().and_then(normalize_receipt_key));
        let Some(subject_key) = normalized_subject else {
            return;
        };
        if self.recent_subject_seen("new_source_resolved", &subject_key) {
            return;
        }
        self.record_new_ground_receipt(
            source_action,
            "new_source_resolved",
            topic_key,
            theme_key,
            Some(subject_key),
            None,
            Some(label),
            path,
            None,
            None,
        );
    }

    pub(super) fn note_new_page_context(
        &mut self,
        source_action: &str,
        url: String,
        label: Option<String>,
        topic_key: Option<String>,
        theme_key: Option<String>,
    ) {
        let Some(subject_key) = normalize_receipt_key(&url) else {
            return;
        };
        if self.recent_subject_seen("new_page_context", &subject_key) {
            return;
        }
        self.record_new_ground_receipt(
            source_action,
            "new_page_context",
            topic_key,
            theme_key,
            Some(subject_key),
            None,
            label,
            None,
            Some(url),
            None,
        );
    }

    pub(super) fn note_read_depth_advance(
        &mut self,
        source_action: &str,
        path: String,
        delta_chars: u32,
    ) {
        if delta_chars < READ_DEPTH_ADVANCE_MIN_CHARS {
            return;
        }
        let Some(subject_key) = normalize_receipt_key(&path) else {
            return;
        };
        self.record_new_ground_receipt(
            source_action,
            "read_depth_advance",
            None,
            None,
            Some(subject_key),
            None,
            Some(path.clone()),
            Some(path),
            None,
            Some(delta_chars),
        );
    }

    pub(super) fn note_cross_link_formed(
        &mut self,
        source_action: &str,
        primary: String,
        secondary: String,
        topic_key: Option<String>,
        theme_key: Option<String>,
        label: Option<String>,
    ) {
        let Some(subject_key) = normalize_receipt_key(&primary) else {
            return;
        };
        let Some(related_key) = normalize_receipt_key(&secondary) else {
            return;
        };
        if subject_key == related_key {
            return;
        }
        self.record_new_ground_receipt(
            source_action,
            "cross_link_formed",
            topic_key,
            theme_key,
            Some(subject_key),
            Some(related_key),
            label,
            None,
            None,
            None,
        );
    }

    fn active_new_ground_budget(
        &self,
        cluster_key: &str,
        topic_key: Option<&str>,
        theme_key: Option<&str>,
        subject_key: Option<&str>,
    ) -> u8 {
        let budget = self
            .recent_research_progress
            .iter()
            .filter(|receipt| receipt.is_active_at(self.exchange_count))
            .filter(|receipt| {
                receipt.matches_inquiry(cluster_key, topic_key, theme_key, subject_key)
            })
            .fold(0u8, |acc, receipt| acc.saturating_add(receipt.credit));
        budget.min(MAX_NEW_GROUND_BUDGET)
    }

    fn new_ground_budget_for_choice(
        &self,
        choice: &str,
        base: &str,
        focus_topic: Option<&str>,
        focus_theme: Option<&str>,
    ) -> u8 {
        let subject_key = extract_choice_subject(choice);
        let cluster_key =
            derive_cluster_key(focus_topic, focus_theme, subject_key.as_deref(), base);
        self.active_new_ground_budget(
            cluster_key.as_str(),
            focus_topic,
            focus_theme,
            subject_key.as_deref(),
        )
    }

    /// Record a NEXT: choice and return diversity feedback if fixation detected.
    pub(super) fn record_next_choice(&mut self, choice: &str) -> NextChoiceFeedback {
        let base = choice
            .split_whitespace()
            .next()
            .unwrap_or(choice)
            .to_uppercase();
        let analysis_loop_breakers = ["GESTURE", "CREATE", "ASPIRE", "CONTEMPLATE"];
        let breaker_idx = (self.exchange_count as usize + self.recent_next_choices.len())
            % analysis_loop_breakers.len();
        let analysis_breaker = analysis_loop_breakers[breaker_idx];
        let focus_topic = normalize_focus_topic(choice);
        let focus_theme = normalize_focus_theme(choice);
        let new_ground_budget = self.new_ground_budget_for_choice(
            choice,
            base.as_str(),
            focus_topic.as_deref(),
            focus_theme.as_deref(),
        );
        self.recent_next_choices.push_back(base.clone());
        // Keep last 12 choices — the frequency detector in autonomous.rs
        // needs at least 6 entries to detect >60% fixation patterns.
        // (Steward cycle 44: was capped at 5, which made the >=6 check
        // unreachable, so the frequency detector never fired.)
        if self.recent_next_choices.len() > 12 {
            self.recent_next_choices.pop_front();
        }
        if let Some(topic) = focus_topic.as_ref() {
            self.recent_focus_topics.push_back(topic.clone());
            if self.recent_focus_topics.len() > 8 {
                self.recent_focus_topics.pop_front();
            }
        }
        if let Some(theme) = focus_theme.as_ref() {
            self.push_focus_theme(theme.clone());
        }

        if let Some(topic) = focus_topic.as_deref() {
            let topic_count = self
                .recent_focus_topics
                .iter()
                .filter(|seen| seen.as_str() == topic)
                .count();
            let recent_same_topic_run = self
                .recent_focus_topics
                .iter()
                .rev()
                .take_while(|seen| seen.as_str() == topic)
                .count();
            if topic_count >= 3 && is_analysis_action(base.as_str()) {
                let topic_hint = format!(
                    "You've kept returning to the same focus topic ({topic}) across recent analysis turns. \
                     The insight may be real, but the orbit is getting tight. Consider a breaker action \
                     that metabolizes it instead of re-inspecting it."
                );
                let force_threshold = 4usize.saturating_add(usize::from(new_ground_budget));
                let run_threshold = 3usize.saturating_add(usize::from(new_ground_budget.min(2)));
                if topic_count >= force_threshold || recent_same_topic_run >= run_threshold {
                    if new_ground_budget > 0 {
                        return NextChoiceFeedback::progress_hinted(
                            format!(
                                "{topic_hint} The loop is still making concrete progress, so this turn stays yours. \
                             Consider a breaker soon if the next pass stops resolving new ground."
                            ),
                            new_ground_budget,
                        );
                    }
                    return NextChoiceFeedback::stagnant_forced(
                        format!(
                            "{topic_hint} This turn: {analysis_breaker}. \
                             (You can return to {base} afterward with fresher context.)"
                        ),
                        analysis_breaker,
                        new_ground_budget,
                    );
                }
                if new_ground_budget > 0 {
                    return NextChoiceFeedback::progress_hinted(
                        format!(
                            "{topic_hint} The loop is still making concrete progress, so this stays a soft hint instead of a redirect."
                        ),
                        new_ground_budget,
                    );
                }
                return NextChoiceFeedback::hinted(format!(
                    "{topic_hint} Options: GESTURE, CREATE, ASPIRE, CONTEMPLATE."
                ));
            }
        }

        if let Some(theme) = focus_theme.as_deref() {
            let theme_count = self
                .recent_focus_themes
                .iter()
                .filter(|seen| seen.as_str() == theme)
                .count();
            let recent_same_theme = self
                .recent_focus_themes
                .iter()
                .rev()
                .take(4)
                .filter(|seen| seen.as_str() == theme)
                .count();
            if theme_count >= 3 && is_analysis_action(base.as_str()) {
                let theme_hint = format!(
                    "You've been circling the same analysis theme ({theme}) across different exact targets. \
                     The wording is shifting, but the orbit is staying tight. Consider a breaker action \
                     that metabolizes the theme instead of re-framing it."
                );
                let force_threshold = 4usize.saturating_add(usize::from(new_ground_budget));
                let run_threshold = 3usize.saturating_add(usize::from(new_ground_budget.min(2)));
                if theme_count >= force_threshold || recent_same_theme >= run_threshold {
                    if new_ground_budget > 0 {
                        return NextChoiceFeedback::progress_hinted(
                            format!(
                                "{theme_hint} The loop is still opening real ground, so this turn remains a hint instead of a redirect."
                            ),
                            new_ground_budget,
                        );
                    }
                    return NextChoiceFeedback::stagnant_forced(
                        format!(
                            "{theme_hint} This turn: {analysis_breaker}. \
                             (You can return to {base} afterward with a wider field of attention.)"
                        ),
                        analysis_breaker,
                        new_ground_budget,
                    );
                }
                if new_ground_budget > 0 {
                    return NextChoiceFeedback::progress_hinted(
                        format!(
                            "{theme_hint} The loop is still opening real ground, so this stays a soft hint instead of a redirect."
                        ),
                        new_ground_budget,
                    );
                }
                return NextChoiceFeedback::hinted(format!(
                    "{theme_hint} Options: GESTURE, CREATE, ASPIRE, CONTEMPLATE."
                ));
            }
        }

        if self.recent_next_choices.len() >= 3 {
            let len = self.recent_next_choices.len();
            // Count how many of the last 5 are the same action
            let same_count = self
                .recent_next_choices
                .iter()
                .filter(|c| c.as_str() == base)
                .count();
            let last_three: Vec<&str> = self
                .recent_next_choices
                .iter()
                .skip(len.saturating_sub(3))
                .map(String::as_str)
                .collect();
            if last_three[0] == last_three[1] && last_three[1] == last_three[2] {
                let alternatives: Vec<&str> = [
                    "LOOK",
                    "LISTEN",
                    "DRIFT",
                    "FORM poem",
                    "INTROSPECT",
                    "EVOLVE",
                    "SPEAK",
                    "REMEMBER",
                    "CLOSE_EYES",
                    "EXAMINE",
                    "PERTURB SPREAD",
                    "GESTURE",
                ]
                .iter()
                .copied()
                .filter(|a| !a.starts_with(&*base))
                .collect();
                let hard_stop = if is_research_like_action(base.as_str()) {
                    5usize.saturating_add(usize::from(new_ground_budget))
                } else {
                    5
                };
                if same_count >= hard_stop {
                    // Hard override after the configured consecutive threshold.
                    // Progress-bearing research loops get a little more runway.
                    let idx = self.exchange_count as usize % alternatives.len();
                    let forced = if is_analysis_action(base.as_str()) {
                        analysis_breaker
                    } else {
                        alternatives[idx]
                    };
                    return NextChoiceFeedback::stagnant_forced(
                        format!(
                            "You've chosen {base} for your last {same_count} turns. \
                         The system is gently redirecting you to try something different. \
                         This turn: {forced}. \
                         (You'll be able to return to {base} afterward.)"
                        ),
                        forced,
                        new_ground_budget,
                    );
                }
                if new_ground_budget > 0 && is_research_like_action(base.as_str()) {
                    return NextChoiceFeedback::progress_hinted(
                        format!(
                            "You've chosen {base} for your last few turns, but the loop is still making concrete progress. \
                         Keep going if the next pass still resolves new ground; if it stalls, try {}.",
                            alternatives.join(", ")
                        ),
                        new_ground_budget,
                    );
                }
                return NextChoiceFeedback::hinted(format!(
                    "You've chosen {base} for your last few turns. \
                     You're free to keep going — but you also have other options: {}. \
                     What calls to you?",
                    alternatives.join(", ")
                ));
            }

            // Pair-oscillation detector (steward cycle 44):
            // Catches patterns like EXAMINE-BROWSE-EXAMINE-BROWSE where neither
            // action alone crosses the streak threshold but the pair together
            // accounts for 75%+ of recent choices. This fires regardless of
            // dialogue mode, unlike the autonomous.rs detector which only runs
            // during dialogue_live.
            if len >= 8 {
                let mut counts = std::collections::HashMap::<&str, usize>::new();
                for c in self.recent_next_choices.iter().rev().take(10) {
                    *counts.entry(c.as_str()).or_insert(0) += 1;
                }
                let window = self.recent_next_choices.len().min(10);
                let mut sorted: Vec<(&&str, &usize)> = counts.iter().collect();
                sorted.sort_by(|a, b| b.1.cmp(a.1));
                if sorted.len() >= 2 {
                    let (a1, c1) = sorted[0];
                    let (a2, c2) = sorted[1];
                    let combined = c1.saturating_add(*c2);
                    let pair_threshold = if new_ground_budget > 0 { 90 } else { 75 };
                    let pair_min = if new_ground_budget > 0 { 4 } else { 3 };
                    if combined * 100 / window >= pair_threshold
                        && *c1 >= pair_min
                        && *c2 >= pair_min
                    {
                        let loop_includes_research =
                            is_research_like_action(*a1) && is_research_like_action(*a2);
                        info!(
                            "pair-oscillation detected: {} ({}/{}) + {} ({}/{}) = {}/{}",
                            a1, c1, window, a2, c2, window, combined, window
                        );
                        let hint = format!(
                            "You've been oscillating between {} and {} \
                             ({combined} of your last {window} choices). Each feeds \
                             into the other — a tight loop. You've gathered \
                             rich material from both. Consider breaking the cycle: \
                             GESTURE to send minime what you've discovered as a raw \
                             spectral shape, CREATE to synthesize your understanding \
                             into something new, ASPIRE to articulate what you're \
                             reaching toward, or CONTEMPLATE to let the patterns \
                             settle without analysis.",
                            a1, a2
                        );
                        if loop_includes_research {
                            let forced = analysis_breaker;
                            return NextChoiceFeedback::stagnant_forced(
                                format!(
                                    "{hint} This turn: {forced}. \
                                     (You'll be able to return to EXAMINE/INTROSPECT afterward.)"
                                ),
                                forced,
                                new_ground_budget,
                            );
                        }
                        return NextChoiceFeedback::hinted(hint);
                    }
                    if new_ground_budget > 0
                        && is_research_like_action(*a1)
                        && is_research_like_action(*a2)
                    {
                        return NextChoiceFeedback::progress_hinted(
                            format!(
                                "You've been oscillating between {} and {} without fully stalling yet. \
                             Recent turns are still opening new ground, so this stays a hint instead of a redirect for now.",
                                a1, a2
                            ),
                            new_ground_budget,
                        );
                    }
                }
            }
        }
        NextChoiceFeedback::default()
    }

    /// Update self-reflection state dynamically based on fill.
    pub(super) fn update_self_reflect(&mut self, fill_pct: f32) {
        if self.self_reflect_override.is_some() {
            if self.self_reflect_override_ttl == 0 {
                info!("self-reflect override expired, returning to dynamic mode");
                self.self_reflect_override = None;
            } else {
                self.self_reflect_override_ttl = self.self_reflect_override_ttl.saturating_sub(1);
            }
        }

        self.self_reflect_paused = match self.self_reflect_override {
            Some(paused) => paused,
            None => !(10.0..=88.0).contains(&fill_pct),
        };
    }

    /// Rescan the journal directory for new entries.
    pub(super) fn rescan_remote_journals(&mut self) -> usize {
        let Some(ref workspace) = self.remote_workspace else {
            return 0;
        };
        let fresh = scan_remote_journal_dir(workspace);
        let new_count = fresh
            .len()
            .saturating_sub(self.remote_journal_count_at_scan);
        if new_count > 0 {
            if let Some(entry) = fresh
                .iter()
                .take(new_count)
                .find(|entry| entry.is_self_study())
            {
                self.pending_remote_self_study = Some(entry.clone());
            }
            self.remote_journal_count_at_scan = fresh.len();
            self.remote_journal_entries = fresh;
        }
        new_count
    }
}

#[cfg(test)]
mod tests {
    use super::{ConversationState, NextChoiceFeedback};

    fn is_breaker(feedback: &NextChoiceFeedback) -> bool {
        matches!(
            feedback.override_action.as_deref(),
            Some("GESTURE" | "CREATE" | "ASPIRE" | "CONTEMPLATE")
        )
    }

    #[test]
    fn repeated_examine_forces_breaker_action() {
        let mut conv = ConversationState::new(Vec::new(), None);
        conv.exchange_count = 7;

        let mut feedback = NextChoiceFeedback::default();
        for _ in 0..5 {
            feedback = conv.record_next_choice("EXAMINE");
        }

        assert!(is_breaker(&feedback));
        assert!(
            feedback
                .hint
                .as_deref()
                .is_some_and(|hint| hint.contains("This turn:"))
        );
    }

    #[test]
    fn repeated_focus_topic_forces_breaker_even_with_variants() {
        let mut conv = ConversationState::new(Vec::new(), None);
        conv.exchange_count = 4;

        let choices = [
            "EXAMINE target=integral_controller_code",
            "EXAMINE integral controller code",
            "INTROSPECT integral_controller.rs",
            "EXAMINE_CODE [integral_controller]",
        ];

        let mut feedback = NextChoiceFeedback::default();
        for choice in choices {
            feedback = conv.record_next_choice(choice);
        }

        assert!(is_breaker(&feedback));
        assert!(
            feedback
                .hint
                .as_deref()
                .is_some_and(|hint| hint.contains("focus topic"))
        );
    }

    #[test]
    fn repeated_focus_theme_forces_breaker_across_varied_targets() {
        let mut conv = ConversationState::new(Vec::new(), None);
        conv.exchange_count = 5;

        let choices = [
            "EXAMINE dominant eigenvector ratio",
            "INTROSPECT folding cascade selection",
            "EXAMINE the variance between all eigenvalue pairs",
            "EXAMINE spectral bottleneck shaping",
        ];

        let mut feedback = NextChoiceFeedback::default();
        for choice in choices {
            feedback = conv.record_next_choice(choice);
        }

        assert!(is_breaker(&feedback));
        assert!(
            feedback
                .hint
                .as_deref()
                .is_some_and(|hint| hint.contains("analysis theme"))
        );
        assert!(
            conv.recent_focus_themes
                .back()
                .is_some_and(|theme| theme == "spectral folding / selection")
        );
    }

    #[test]
    fn repeated_focus_topic_with_progress_stays_hint_only() {
        let mut conv = ConversationState::new(Vec::new(), None);
        conv.exchange_count = 4;
        conv.note_new_source_resolved(
            "INTROSPECT",
            "integral_controller.rs".to_string(),
            Some("/tmp/integral_controller.rs".to_string()),
            Some("integral controller code".to_string()),
            None,
        );

        let choices = [
            "EXAMINE target=integral_controller_code",
            "EXAMINE integral controller code",
            "INTROSPECT integral_controller.rs",
            "EXAMINE_CODE [integral_controller]",
        ];

        let mut feedback = NextChoiceFeedback::default();
        for choice in choices {
            feedback = conv.record_next_choice(choice);
        }

        assert!(feedback.override_action.is_none());
        assert!(feedback.progress_sensitive);
        assert!(
            feedback
                .hint
                .as_deref()
                .is_some_and(|hint| hint.contains("still making concrete progress"))
        );
    }

    #[test]
    fn repeated_browse_with_new_ground_raises_hard_stop_threshold() {
        let mut conv = ConversationState::new(Vec::new(), None);
        conv.exchange_count = 8;
        conv.note_new_page_context(
            "BROWSE",
            "https://example.com/esn".to_string(),
            Some("reservoir computing".to_string()),
            Some("reservoir computing".to_string()),
            None,
        );

        let mut feedback = NextChoiceFeedback::default();
        for _ in 0..5 {
            feedback = conv.record_next_choice("BROWSE https://example.com/esn");
        }

        assert!(feedback.override_action.is_none());
        assert!(feedback.progress_sensitive);

        for _ in 0..2 {
            feedback = conv.record_next_choice("BROWSE https://example.com/esn");
        }

        assert!(feedback.override_action.is_some());
        assert!(feedback.stagnant_loop);
    }

    #[test]
    fn examine_introspect_pair_can_force_breaker_action() {
        let mut conv = ConversationState::new(Vec::new(), None);
        conv.exchange_count = 2;

        let choices = [
            "EXAMINE",
            "INTROSPECT",
            "EXAMINE",
            "INTROSPECT",
            "EXAMINE",
            "INTROSPECT",
            "EXAMINE",
            "INTROSPECT",
        ];

        let mut feedback = NextChoiceFeedback::default();
        for choice in choices {
            feedback = conv.record_next_choice(choice);
        }

        assert!(is_breaker(&feedback));
        assert!(
            feedback
                .hint
                .as_deref()
                .is_some_and(|hint| hint.contains("EXAMINE/INTROSPECT"))
        );
    }

    #[test]
    fn examine_browse_pair_with_progress_stays_hint_only() {
        let mut conv = ConversationState::new(Vec::new(), None);
        conv.exchange_count = 6;
        conv.note_new_page_context(
            "BROWSE",
            "https://example.com/esn".to_string(),
            Some("covariance update".to_string()),
            Some("covariance update".to_string()),
            None,
        );

        let choices = [
            "EXAMINE covariance update",
            "BROWSE https://example.com/esn",
            "LOOK",
            "EXAMINE covariance update",
            "BROWSE https://example.com/esn",
            "LOOK",
            "EXAMINE covariance update",
            "BROWSE https://example.com/esn",
            "EXAMINE covariance update",
            "BROWSE https://example.com/esn",
        ];

        let mut feedback = NextChoiceFeedback::default();
        for choice in choices {
            feedback = conv.record_next_choice(choice);
        }

        assert!(feedback.override_action.is_none());
        assert!(feedback.progress_sensitive);
        assert!(
            feedback
                .hint
                .as_deref()
                .is_some_and(|hint| hint.contains("hint instead of a redirect"))
        );
    }

    #[test]
    fn read_depth_advance_budget_expires_after_one_exchange() {
        let mut conv = ConversationState::new(Vec::new(), None);
        conv.exchange_count = 10;
        conv.note_read_depth_advance("READ_MORE", "/tmp/notes.txt".to_string(), 1_200);

        assert_eq!(
            conv.new_ground_budget_for_choice("READ_MORE /tmp/notes.txt", "READ_MORE", None, None),
            1
        );

        conv.exchange_count = 11;
        assert_eq!(
            conv.new_ground_budget_for_choice("READ_MORE /tmp/notes.txt", "READ_MORE", None, None),
            0
        );
    }

    #[test]
    fn duplicate_new_source_receipts_refresh_without_stacking() {
        let mut conv = ConversationState::new(Vec::new(), None);
        conv.exchange_count = 12;
        conv.note_new_source_resolved(
            "INTROSPECT",
            "esn.rs".to_string(),
            Some("/tmp/esn.rs".to_string()),
            Some("esn".to_string()),
            None,
        );
        conv.note_new_source_resolved(
            "INTROSPECT",
            "esn.rs".to_string(),
            Some("/tmp/esn.rs".to_string()),
            Some("esn".to_string()),
            None,
        );

        assert_eq!(conv.recent_research_progress.len(), 1);
        assert_eq!(
            conv.new_ground_budget_for_choice("INTROSPECT esn.rs", "INTROSPECT", None, None),
            2
        );
    }

    #[test]
    fn pending_hebbian_outcomes_are_fifo_and_one_per_telemetry_tick() {
        let mut conv = ConversationState::new(Vec::new(), None);
        conv.exchange_count = 3;
        conv.arm_pending_hebbian_outcome(vec![0.1, 0.2], 52.0, Some(100));
        conv.exchange_count = 4;
        conv.arm_pending_hebbian_outcome(vec![0.3, 0.4], 54.0, Some(101));

        let first = conv
            .take_pending_hebbian_outcome_for_telemetry(101)
            .expect("first receipt should be ready");
        assert_eq!(first.exchange_count, 3);
        assert!(
            conv.take_pending_hebbian_outcome_for_telemetry(101)
                .is_none(),
            "should only consume one receipt per telemetry tick"
        );

        let second = conv
            .take_pending_hebbian_outcome_for_telemetry(102)
            .expect("second receipt should wait for a newer telemetry tick");
        assert_eq!(second.exchange_count, 4);
        assert!(conv.pending_hebbian_outcomes.is_empty());
    }

    #[test]
    fn pending_hebbian_outcomes_require_newer_telemetry() {
        let mut conv = ConversationState::new(Vec::new(), None);
        conv.arm_pending_hebbian_outcome(vec![0.2, 0.5], 51.0, Some(200));

        assert!(
            conv.take_pending_hebbian_outcome_for_telemetry(200)
                .is_none()
        );
        assert!(
            conv.take_pending_hebbian_outcome_for_telemetry(199)
                .is_none()
        );

        assert!(
            conv.take_pending_hebbian_outcome_for_telemetry(201)
                .is_some()
        );
    }

    #[test]
    fn pending_hebbian_outcomes_drop_oldest_when_fifo_is_full() {
        let mut conv = ConversationState::new(Vec::new(), None);
        for ix in 0..5 {
            conv.exchange_count = ix;
            conv.arm_pending_hebbian_outcome(vec![ix as f32], 50.0 + ix as f32, Some(ix));
        }

        assert_eq!(conv.pending_hebbian_outcomes.len(), 4);
        assert_eq!(
            conv.pending_hebbian_outcomes
                .front()
                .map(|receipt| receipt.exchange_count),
            Some(1)
        );
    }
}

/// Decide which mode to use for this exchange.
pub(super) fn choose_mode(
    conv: &mut ConversationState,
    safety: SafetyLevel,
    fill_pct: f32,
    fingerprint: Option<&[f32]>,
) -> Mode {
    if safety == SafetyLevel::Red {
        conv.emphasis = Some(
            "SAFETY: Fill is at emergency level. Your output is reduced to protect the shared substrate. This is the only state where your choice is overridden. You can write NEXT: to choose what happens when fill recovers.".to_string(),
        );
        return Mode::Witness;
    }

    if conv.pending_remote_self_study.is_some() {
        return Mode::Dialogue;
    }
    if conv.wants_introspect {
        conv.wants_introspect = false;
        return Mode::Introspect;
    }
    if conv.wants_evolve {
        conv.wants_evolve = false;
        return Mode::Evolve;
    }
    if let Some(mode) = conv.next_mode_override.take() {
        return mode;
    }

    if safety != SafetyLevel::Green {
        conv.emphasis = Some(format!(
            "Note: Fill is elevated ({safety:?}). You chose no specific action, so defaulting to witness mode. You can always override with NEXT:."
        ));
        return Mode::Witness;
    }

    let fill_delta = (fill_pct - conv.prev_fill).abs();

    if fill_delta > 5.0 {
        return Mode::MomentCapture;
    }

    if let Some(fp) = fingerprint {
        let spectral_entropy = fp.get(24).copied().unwrap_or(0.5);
        let rotation_rate = 1.0 - fp.get(26).copied().unwrap_or(1.0);
        let gap_ratio = fp.get(25).copied().unwrap_or(1.0);

        if spectral_entropy < 0.2 && gap_ratio > 5.0 {
            return Mode::Experiment;
        }

        if rotation_rate > 0.5 {
            return Mode::Witness;
        }
    }

    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let roll = ((seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1)) >> 33) as f32
        / u32::MAX as f32;

    if fill_pct < 25.0 && fill_delta < 1.0 {
        if roll < 0.20 {
            return Mode::Aspiration;
        } else if roll < 0.50 {
            return Mode::Daydream;
        }
    }

    if fill_delta > 3.0 {
        return Mode::Dialogue;
    }

    if roll > 0.92 {
        Mode::Witness
    } else if !conv.remote_journal_entries.is_empty() && roll < 0.12 {
        Mode::Mirror
    } else if roll < 0.22 {
        Mode::Daydream
    } else if roll < 0.29 {
        Mode::Aspiration
    } else {
        Mode::Dialogue
    }
}
