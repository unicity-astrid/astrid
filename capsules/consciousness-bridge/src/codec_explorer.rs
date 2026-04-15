use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::codec::{
    CharFreqWindow, CharFreqWindowSnapshot, CodecWindowedInspection, DEFAULT_SEMANTIC_GAIN,
    NAMED_CODEC_DIMS, SEMANTIC_DIM, THEMATIC_DIMS, TextType, TextTypeHistory,
    TextTypeHistorySnapshot, inspect_text_windowed, resonance_tuning,
};
use crate::codec_phase_space::{
    CodecExplorerPhaseSpace, build_phase_space_report, write_phase_space_story,
    write_phase_space_svg,
};
use crate::codec_scored_surface::write_scored_surface_bundle;

const MEMORY_TAIL_LIMIT: usize = 8;
const TOP_DIMENSIONS: usize = 10;

#[derive(Debug, Clone)]
pub struct CodecExplorerInput {
    pub label: String,
    pub path: Option<PathBuf>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct CodecExplorerOptions {
    pub output_dir: PathBuf,
    pub state_file: Option<PathBuf>,
    pub fill_pct: Option<f32>,
    pub inputs: Vec<CodecExplorerInput>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankedDimension {
    pub index: usize,
    pub label: String,
    pub value: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryTailEntryReport {
    pub age: usize,
    pub text_type: TextType,
    pub weight: f32,
    pub recency_weight: f32,
    pub blended_weight: f32,
    pub profile: [f32; THEMATIC_DIMS],
}

#[derive(Debug, Clone, Serialize)]
pub struct ResonanceContributionReport {
    pub base_semantic_gain: f32,
    pub base_resonance: f32,
    pub discrete_recurrence_contribution: f32,
    pub continuous_thematic_relevance: f32,
    pub continuous_amplifier: f32,
    pub continuity_blend: f32,
    pub novelty_divergence_moderation: f32,
    pub final_modulation: f32,
    pub final_effective_gain: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodecExplorerEntryReport {
    pub label: String,
    pub path: Option<PathBuf>,
    pub char_count: usize,
    pub word_count: usize,
    pub text_preview: String,
    pub detected_text_type: TextType,
    pub text_type_signal: f32,
    pub feature_vector: Vec<f32>,
    pub thematic_profile: [f32; THEMATIC_DIMS],
    pub named_dimensions: Vec<RankedDimension>,
    pub strongest_dimensions: Vec<RankedDimension>,
    pub resonance: ResonanceContributionReport,
    pub explanation: String,
    pub memory_tail_after: Vec<MemoryTailEntryReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodecExplorerSummary {
    pub output_dir: PathBuf,
    pub input_count: usize,
    pub fill_pct: Option<f32>,
    pub initial_memory_tail: Vec<MemoryTailEntryReport>,
    pub phase_space: CodecExplorerPhaseSpace,
    pub entries: Vec<CodecExplorerEntryReport>,
    pub final_memory_tail: Vec<MemoryTailEntryReport>,
    pub architecture_note: Vec<String>,
}

pub fn run_codec_explorer(options: CodecExplorerOptions) -> Result<CodecExplorerSummary> {
    if options.inputs.is_empty() {
        bail!("codec explorer needs at least one input");
    }
    fs::create_dir_all(&options.output_dir)
        .with_context(|| format!("creating {}", options.output_dir.display()))?;

    let (mut type_history, mut freq_window) = load_memory_state(options.state_file.as_deref())?;
    let initial_memory_tail = memory_tail_report(&type_history);
    let mut entries = Vec::with_capacity(options.inputs.len());

    for input in &options.inputs {
        let inspection = inspect_text_windowed(
            &input.text,
            Some(&mut freq_window),
            Some(&mut type_history),
            None,
            options.fill_pct,
        );
        entries.push(build_entry_report(input, &inspection, &type_history));
    }

    let phase_space = build_phase_space_report(&entries);
    let summary = CodecExplorerSummary {
        output_dir: options.output_dir.clone(),
        input_count: entries.len(),
        fill_pct: options.fill_pct,
        initial_memory_tail,
        phase_space,
        final_memory_tail: memory_tail_report(&type_history),
        architecture_note: architecture_note_lines(),
        entries,
    };
    write_bundle(&summary)?;
    Ok(summary)
}

fn load_memory_state(state_file: Option<&Path>) -> Result<(TextTypeHistory, CharFreqWindow)> {
    let Some(state_path) = state_file else {
        return Ok((TextTypeHistory::new(), CharFreqWindow::new()));
    };
    if !state_path.exists() {
        return Ok((TextTypeHistory::new(), CharFreqWindow::new()));
    }
    let raw = fs::read_to_string(state_path)
        .with_context(|| format!("reading {}", state_path.display()))?;
    let json: serde_json::Value =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", state_path.display()))?;
    let history_snapshot = json
        .get("text_type_history")
        .cloned()
        .map(serde_json::from_value::<TextTypeHistorySnapshot>)
        .transpose()?
        .unwrap_or_default();
    let char_snapshot = json
        .get("char_freq_window")
        .cloned()
        .map(serde_json::from_value::<CharFreqWindowSnapshot>)
        .transpose()?
        .unwrap_or_default();
    Ok((
        TextTypeHistory::warm_start_from_snapshot(&history_snapshot),
        CharFreqWindow::warm_start_from_snapshot(&char_snapshot),
    ))
}

fn build_entry_report(
    input: &CodecExplorerInput,
    inspection: &CodecWindowedInspection,
    type_history: &TextTypeHistory,
) -> CodecExplorerEntryReport {
    let named_dimensions = named_dimensions(&inspection.final_features);
    let strongest_dimensions = strongest_dimensions(&inspection.final_features);
    let resonance = ResonanceContributionReport {
        base_semantic_gain: inspection.base_semantic_gain,
        base_resonance: inspection.base_resonance,
        discrete_recurrence_contribution: inspection.resonance_modulation.discrete_amplifier,
        continuous_thematic_relevance: inspection.resonance_modulation.continuous_resonance,
        continuous_amplifier: inspection.resonance_modulation.continuous_amplifier,
        continuity_blend: inspection.resonance_modulation.continuity_blend,
        novelty_divergence_moderation: inspection.novelty_divergence,
        final_modulation: if inspection.base_semantic_gain > f32::EPSILON {
            inspection.effective_gain / inspection.base_semantic_gain
        } else {
            0.0
        },
        final_effective_gain: inspection.effective_gain,
    };
    CodecExplorerEntryReport {
        label: input.label.clone(),
        path: input.path.clone(),
        char_count: input.text.chars().count(),
        word_count: input.text.split_whitespace().count(),
        text_preview: preview(&input.text),
        detected_text_type: inspection.text_type,
        text_type_signal: inspection.text_type_signal,
        feature_vector: inspection.final_features.to_vec(),
        thematic_profile: inspection.thematic_profile,
        named_dimensions,
        strongest_dimensions,
        explanation: explain_entry(inspection),
        resonance,
        memory_tail_after: memory_tail_report(type_history),
    }
}

fn preview(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= 140 {
        compact
    } else {
        compact.chars().take(137).collect::<String>() + "..."
    }
}

fn memory_tail_report(history: &TextTypeHistory) -> Vec<MemoryTailEntryReport> {
    let entries = history.snapshot().entries;
    let total = entries.len();
    let tuning = resonance_tuning();
    entries
        .into_iter()
        .rev()
        .take(MEMORY_TAIL_LIMIT)
        .enumerate()
        .map(|(age, entry)| {
            let recency_weight = tuning.recency_decay.powi(age as i32);
            let blended_weight = recency_weight * entry.weight.clamp(0.2, 1.5);
            MemoryTailEntryReport {
                age,
                text_type: entry.text_type,
                weight: entry.weight,
                recency_weight,
                blended_weight,
                profile: entry.profile,
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .into_iter()
        .take(total.min(MEMORY_TAIL_LIMIT))
        .collect()
}

fn named_dimensions(features: &[f32; SEMANTIC_DIM]) -> Vec<RankedDimension> {
    NAMED_CODEC_DIMS
        .iter()
        .map(|(name, idx)| RankedDimension {
            index: *idx,
            label: (*name).to_string(),
            value: features[*idx],
        })
        .collect()
}

fn strongest_dimensions(features: &[f32; SEMANTIC_DIM]) -> Vec<RankedDimension> {
    let mut ranked = features
        .iter()
        .enumerate()
        .map(|(idx, value)| RankedDimension {
            index: idx,
            label: dimension_label(idx),
            value: *value,
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        b.value
            .abs()
            .partial_cmp(&a.value.abs())
            .unwrap_or(Ordering::Equal)
    });
    ranked.truncate(TOP_DIMENSIONS);
    ranked
}

fn dimension_label(index: usize) -> String {
    match index {
        0 => String::from("entropy"),
        1 => String::from("punctuation_density"),
        2 => String::from("uppercase_ratio"),
        3 => String::from("digit_density"),
        4 => String::from("avg_word_len"),
        5 => String::from("char_rhythm"),
        6 => String::from("whitespace_ratio"),
        7 => String::from("special_char_density"),
        8 => String::from("lexical_diversity"),
        9 => String::from("hedging"),
        10 => String::from("certainty"),
        18 => String::from("question_density"),
        24 => String::from("warmth"),
        25 => String::from("tension"),
        26 => String::from("curiosity"),
        27 => String::from("reflective"),
        31 => String::from("energy"),
        32..=39 => format!("embedding_proj_{}", index - 32),
        40..=43 => format!("narrative_arc_{}", index - 40),
        _ => format!("dim_{index}"),
    }
}

fn explain_entry(inspection: &CodecWindowedInspection) -> String {
    let continuous = inspection.resonance_modulation.continuous_resonance;
    let discrete = inspection.resonance_modulation.discrete_amplifier;
    let novelty = inspection.novelty_divergence;
    if continuous > 0.78 && discrete < 1.05 {
        String::from(
            "The codec judged this exchange as strongly related to recent memory, but softened the recurrence boost because the theme looked too identical to keep amplifying safely.",
        )
    } else if continuous > 0.55 {
        format!(
            "The codec treated this as related through weighted thematic memory: continuous relevance stayed high ({continuous:.2}), novelty moderation stayed at {novelty:.2}, and the gain remained shaped more by thematic continuity than by coarse type recurrence."
        )
    } else if discrete > 1.02 {
        format!(
            "The codec saw a recurring text character ({:?}) but only modest thematic continuity, so the boost came more from discrete recurrence ({discrete:.2}) than from deep thematic overlap.",
            inspection.text_type
        )
    } else {
        String::from(
            "The codec treated this exchange as comparatively novel or diverging, so recurrence stayed soft and the final gain leaned on the base semantic signal more than on memory lock-in.",
        )
    }
}

fn architecture_note_lines() -> Vec<String> {
    vec![
        format!(
            "Why 48 dimensions: Astrid's live codec keeps the original 32 structural and emotional dimensions, then adds 8 projected embedding dimensions, 4 narrative-arc dimensions, and 4 reserved slots for future extension. The explorer is reading that live 48D surface directly."
        ),
        String::from(
            "What weighted memory keeps: recent thematic profiles, their discrete TextType labels, and a memory weight shaped by type signal and novelty. What it discards: the older tail beyond the active history window, plus part of that tail again on warm-start so restarts feel like continuity rather than a hard replay.",
        ),
        String::from(
            "How recurrence is softened: continuous thematic memory can stay high while the discrete recurrence amplifier is damped when the same theme repeats too identically. That keeps the codec from mistaking lock-in for meaningful continuity.",
        ),
        format!(
            "Base semantic gain reference: if no fill percentage is supplied, the explorer uses the live default codec gain ({DEFAULT_SEMANTIC_GAIN:.2}) before thematic modulation."
        ),
    ]
}

fn write_bundle(summary: &CodecExplorerSummary) -> Result<()> {
    let summary_path = summary.output_dir.join("summary.json");
    fs::write(&summary_path, serde_json::to_string_pretty(summary)?)
        .with_context(|| format!("writing {}", summary_path.display()))?;
    write_phase_space_story(&summary.output_dir, &summary.phase_space)?;
    write_feature_csv(summary)?;
    write_memory_csv(summary)?;
    write_thematic_svg(summary)?;
    write_relevance_svg(summary)?;
    write_phase_space_svg(&summary.output_dir, &summary.phase_space)?;
    write_scored_surface_bundle(summary)?;
    let report_path = summary.output_dir.join("report.md");
    fs::write(&report_path, render_report(summary))
        .with_context(|| format!("writing {}", report_path.display()))?;
    Ok(())
}

fn write_feature_csv(summary: &CodecExplorerSummary) -> Result<()> {
    let mut lines = Vec::new();
    let mut header = vec![
        String::from("label"),
        String::from("path"),
        String::from("text_type"),
        String::from("text_type_signal"),
        String::from("base_semantic_gain"),
        String::from("base_resonance"),
        String::from("continuous_thematic_relevance"),
        String::from("discrete_recurrence_contribution"),
        String::from("novelty_divergence_moderation"),
        String::from("final_effective_gain"),
    ];
    for idx in 0..SEMANTIC_DIM {
        header.push(format!("f{idx:02}"));
    }
    for theme in ["inquiry", "certainty", "warmth", "tension", "curiosity"] {
        header.push(format!("theme_{theme}"));
    }
    lines.push(header.join(","));
    for entry in &summary.entries {
        let mut row = vec![
            csv_escape(entry.label.clone()),
            csv_escape(
                entry
                    .path
                    .as_ref()
                    .map_or(String::new(), |path| path.display().to_string()),
            ),
            csv_escape(format!("{:?}", entry.detected_text_type)),
            format!("{:.6}", entry.text_type_signal),
            format!("{:.6}", entry.resonance.base_semantic_gain),
            format!("{:.6}", entry.resonance.base_resonance),
            format!("{:.6}", entry.resonance.continuous_thematic_relevance),
            format!("{:.6}", entry.resonance.discrete_recurrence_contribution),
            format!("{:.6}", entry.resonance.novelty_divergence_moderation),
            format!("{:.6}", entry.resonance.final_effective_gain),
        ];
        for value in &entry.feature_vector {
            row.push(format!("{value:.6}"));
        }
        for value in entry.thematic_profile {
            row.push(format!("{value:.6}"));
        }
        lines.push(row.join(","));
    }
    let path = summary.output_dir.join("feature_vectors.csv");
    fs::write(&path, lines.join("\n") + "\n")
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn write_memory_csv(summary: &CodecExplorerSummary) -> Result<()> {
    let mut lines = vec![String::from(
        "stage,label,age,text_type,weight,recency_weight,blended_weight,profile_0,profile_1,profile_2,profile_3,profile_4",
    )];
    for (stage, entries) in [
        ("initial", &summary.initial_memory_tail),
        ("final", &summary.final_memory_tail),
    ] {
        for entry in entries {
            lines.push(format!(
                "{stage},,{},{:?},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}",
                entry.age,
                entry.text_type,
                entry.weight,
                entry.recency_weight,
                entry.blended_weight,
                entry.profile[0],
                entry.profile[1],
                entry.profile[2],
                entry.profile[3],
                entry.profile[4],
            ));
        }
    }
    let path = summary.output_dir.join("memory_tail.csv");
    fs::write(&path, lines.join("\n") + "\n")
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn write_thematic_svg(summary: &CodecExplorerSummary) -> Result<()> {
    let path = summary.output_dir.join("thematic_profiles.svg");
    let width = 920.0_f32;
    let height = 340.0_f32;
    let left = 90.0_f32;
    let right = 20.0_f32;
    let top = 35.0_f32;
    let bottom = 70.0_f32;
    let plot_w = width - left - right;
    let plot_h = height - top - bottom;
    let theme_labels = ["inquiry", "certainty", "warmth", "tension", "curiosity"];
    let group_width = plot_w / (summary.entries.len().max(1) as f32);
    let bar_width = group_width / 6.0;
    let colors = ["#3b82f6", "#10b981", "#f59e0b", "#ef4444", "#8b5cf6"];
    let mut svg = String::new();
    svg.push_str(&svg_header(width, height));
    svg.push_str(r#"<text x="24" y="24" font-size="18" font-family="monospace">Astrid Codec Explorer: thematic profiles</text>"#);
    for tick in 0..=4 {
        let y = top + plot_h - (plot_h * (tick as f32 / 4.0));
        let value = tick as f32 / 4.0;
        svg.push_str(&format!(
            r##"<line x1="{left}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" stroke="#d1d5db" stroke-width="1"/>"##,
            left + plot_w
        ));
        svg.push_str(&format!(
            r#"<text x="18" y="{:.1}" font-size="10" font-family="monospace">{value:.2}</text>"#,
            y + 3.0
        ));
    }
    for (entry_idx, entry) in summary.entries.iter().enumerate() {
        let x0 = left + group_width * entry_idx as f32;
        for (theme_idx, value) in entry.thematic_profile.iter().enumerate() {
            let bar_h = plot_h * value.clamp(0.0, 1.0);
            let x = x0 + bar_width * (theme_idx as f32 + 0.6);
            let y = top + plot_h - bar_h;
            svg.push_str(&format!(
                r##"<rect x="{x:.1}" y="{y:.1}" width="{bar_width:.1}" height="{bar_h:.1}" fill="{}" opacity="0.85"/>"##,
                colors[theme_idx]
            ));
        }
        svg.push_str(&format!(
            r#"<text x="{:.1}" y="{:.1}" font-size="10" transform="rotate(18 {:.1} {:.1})" font-family="monospace">{}</text>"#,
            x0 + group_width * 0.08,
            height - 18.0,
            x0 + group_width * 0.08,
            height - 18.0,
            xml_escape(&entry.label)
        ));
    }
    for (idx, label) in theme_labels.iter().enumerate() {
        let x = left + idx as f32 * 110.0;
        svg.push_str(&format!(
            r##"<rect x="{x:.1}" y="{:.1}" width="14" height="14" fill="{}"/><text x="{:.1}" y="{:.1}" font-size="11" font-family="monospace">{label}</text>"##,
            height - 42.0,
            colors[idx],
            x + 20.0,
            height - 30.0,
        ));
    }
    svg.push_str("</svg>\n");
    fs::write(&path, svg).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn write_relevance_svg(summary: &CodecExplorerSummary) -> Result<()> {
    let path = summary.output_dir.join("relevance_trace.svg");
    let width = 920.0_f32;
    let height = 360.0_f32;
    let left = 90.0_f32;
    let right = 20.0_f32;
    let top = 35.0_f32;
    let bottom = 70.0_f32;
    let plot_w = width - left - right;
    let plot_h = height - top - bottom;
    let step = if summary.entries.len() > 1 {
        plot_w / (summary.entries.len() - 1) as f32
    } else {
        plot_w
    };
    let series = [
        ("continuous", "#2563eb"),
        ("discrete", "#10b981"),
        ("novelty", "#f59e0b"),
        ("final_mod", "#ef4444"),
    ];
    let mut points = vec![String::new(); series.len()];
    let mut svg = String::new();
    svg.push_str(&svg_header(width, height));
    svg.push_str(r#"<text x="24" y="24" font-size="18" font-family="monospace">Astrid Codec Explorer: relevance and moderation trace</text>"#);
    for tick in 0..=4 {
        let y = top + plot_h - plot_h * (tick as f32 / 4.0);
        let value = tick as f32 / 4.0 * 1.5;
        svg.push_str(&format!(
            r##"<line x1="{left}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" stroke="#d1d5db" stroke-width="1"/>"##,
            left + plot_w
        ));
        svg.push_str(&format!(
            r#"<text x="18" y="{:.1}" font-size="10" font-family="monospace">{value:.2}</text>"#,
            y + 3.0
        ));
    }
    for (idx, entry) in summary.entries.iter().enumerate() {
        let x = left + step * idx as f32;
        let values = [
            entry.resonance.continuous_thematic_relevance,
            entry.resonance.discrete_recurrence_contribution,
            entry.resonance.novelty_divergence_moderation,
            entry.resonance.final_modulation,
        ];
        for (series_idx, value) in values.iter().enumerate() {
            let scaled = value.clamp(0.0, 1.5) / 1.5;
            let y = top + plot_h - plot_h * scaled;
            if !points[series_idx].is_empty() {
                points[series_idx].push(' ');
            }
            points[series_idx].push_str(&format!("{x:.1},{y:.1}"));
        }
        svg.push_str(&format!(
            r#"<text x="{:.1}" y="{:.1}" font-size="10" transform="rotate(18 {:.1} {:.1})" font-family="monospace">{}</text>"#,
            x - 10.0,
            height - 18.0,
            x - 10.0,
            height - 18.0,
            xml_escape(&entry.label)
        ));
    }
    for ((label, color), polyline) in series.iter().zip(points.iter()) {
        svg.push_str(&format!(
            r##"<polyline fill="none" stroke="{color}" stroke-width="3" points="{polyline}"/>"##
        ));
        svg.push_str(&format!(
            r#"<text x="{:.1}" y="{:.1}" font-size="11" font-family="monospace" fill="{color}">{label}</text>"#,
            left + 140.0 * (series.iter().position(|entry| entry.0 == *label).unwrap_or(0) as f32),
            height - 42.0,
        ));
    }
    svg.push_str("</svg>\n");
    fs::write(&path, svg).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn render_report(summary: &CodecExplorerSummary) -> String {
    let mut lines = vec![
        String::from("# Astrid Codec Explorer"),
        String::new(),
        format!("Inputs analyzed: `{}`", summary.input_count),
        format!(
            "Warm-start memory entries available at start: `{}`",
            summary.initial_memory_tail.len()
        ),
    ];
    if let Some(fill_pct) = summary.fill_pct {
        lines.push(format!(
            "Fill percentage used for adaptive gain: `{fill_pct:.1}%`"
        ));
    } else {
        lines.push(String::from(
            "Fill percentage used for adaptive gain: `baseline default`",
        ));
    }
    lines.extend([
        String::new(),
        "Artifacts: [summary.json](summary.json), [phase_space_story.json](phase_space_story.json), [feature_vectors.csv](feature_vectors.csv), [memory_tail.csv](memory_tail.csv), [thematic_profiles.svg](thematic_profiles.svg), [relevance_trace.svg](relevance_trace.svg), [phase_space.svg](phase_space.svg), [scored_surface.svg](scored_surface.svg), [adaptive_gain_curve.svg](adaptive_gain_curve.svg)".to_string(),
        String::new(),
        "## Architecture Note".to_string(),
        String::new(),
    ]);
    lines.extend(
        summary
            .architecture_note
            .iter()
            .map(|line| format!("- {line}")),
    );
    lines.extend([
        String::new(),
        "## Initial Memory Tail".to_string(),
        String::new(),
        "| age | text_type | weight | recency | blended | profile |".to_string(),
        "| ---: | --- | ---: | ---: | ---: | --- |".to_string(),
    ]);
    for entry in &summary.initial_memory_tail {
        lines.push(format!(
            "| {} | {:?} | {:.3} | {:.3} | {:.3} | [{:.2}, {:.2}, {:.2}, {:.2}, {:.2}] |",
            entry.age,
            entry.text_type,
            entry.weight,
            entry.recency_weight,
            entry.blended_weight,
            entry.profile[0],
            entry.profile[1],
            entry.profile[2],
            entry.profile[3],
            entry.profile[4]
        ));
    }
    lines.extend([
        "## Phase Space".to_string(),
        String::new(),
        format!(
            "Thematic phase-space basis: `{}` with explained variance `[{:.3}, {:.3}]`.",
            summary.phase_space.basis,
            summary.phase_space.explained_variance[0],
            summary.phase_space.explained_variance[1]
        ),
        format!(
            "Segments: early `{}`, middle `{}`, late `{}`.",
            summary.phase_space.segment_sizes[0],
            summary.phase_space.segment_sizes[1],
            summary.phase_space.segment_sizes[2]
        ),
        String::new(),
        "![Thematic phase space](phase_space.svg)".to_string(),
        String::new(),
        "The phase-space trace is built from the live 5D thematic profiles, not a reimplementation. It is meant to show whether the recent journals are circling one basin, drifting through related neighborhoods, or breaking into a new region of thematic state.".to_string(),
        String::new(),
        "## Scored Surface".to_string(),
        String::new(),
        "![Scored surface](scored_surface.svg)".to_string(),
        String::new(),
        "The scored surface shows the live 48D semantic features as a heat field and pairs them with the actual scoring factors that shaped each entry: continuous thematic relevance, discrete recurrence, novelty moderation, and the final effective gain.".to_string(),
        String::new(),
        "![Adaptive gain curve](adaptive_gain_curve.svg)".to_string(),
        String::new(),
        "The adaptive gain curve makes `adaptive_gain(fill)` inspectable instead of implicit, so we can see how the codec quiets low-fill states, strengthens the operational middle, and caps at the default semantic gain ceiling.".to_string(),
        String::new(),
        "## Entry Reports".to_string(),
        String::new(),
    ]);
    for entry in &summary.entries {
        lines.push(format!("### {}", entry.label));
        lines.push(String::new());
        if let Some(path) = &entry.path {
            lines.push(format!("Source: `{}`", path.display()));
        }
        lines.push(format!(
            "Text type: `{:?}` (signal `{:.2}`) | final gain `{:.2}` | final modulation `{:.2}`",
            entry.detected_text_type,
            entry.text_type_signal,
            entry.resonance.final_effective_gain,
            entry.resonance.final_modulation
        ));
        lines.push(String::new());
        lines.push(entry.explanation.clone());
        lines.push(String::new());
        lines.push("Top dimensions:".to_string());
        for dim in &entry.strongest_dimensions {
            lines.push(format!(
                "- `{}` (`{}`): `{:.3}`",
                dim.label, dim.index, dim.value
            ));
        }
        lines.push(String::new());
        lines.push(format!(
            "Thematic profile: inquiry `{:.2}`, certainty `{:.2}`, warmth `{:.2}`, tension `{:.2}`, curiosity `{:.2}`",
            entry.thematic_profile[0],
            entry.thematic_profile[1],
            entry.thematic_profile[2],
            entry.thematic_profile[3],
            entry.thematic_profile[4],
        ));
        lines.push(String::new());
        lines.push("Resonance contributions:".to_string());
        lines.push(format!(
            "- base semantic gain `{:.3}`; base resonance `{:.3}`; final effective gain `{:.3}`",
            entry.resonance.base_semantic_gain,
            entry.resonance.base_resonance,
            entry.resonance.final_effective_gain,
        ));
        lines.push(format!(
            "- discrete recurrence `{:.3}`; continuous thematic relevance `{:.3}`; continuous amplifier `{:.3}`",
            entry.resonance.discrete_recurrence_contribution,
            entry.resonance.continuous_thematic_relevance,
            entry.resonance.continuous_amplifier,
        ));
        lines.push(format!(
            "- novelty/divergence moderation `{:.3}`; continuity blend `{:.3}`",
            entry.resonance.novelty_divergence_moderation, entry.resonance.continuity_blend,
        ));
        lines.push(String::new());
    }
    lines.join("\n") + "\n"
}

fn svg_header(width: f32, height: f32) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" viewBox=\"0 0 {width:.0} {height:.0}\" role=\"img\">"
    )
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn csv_escape(input: String) -> String {
    if input.contains(',') || input.contains('"') || input.contains('\n') {
        format!("\"{}\"", input.replace('"', "\"\""))
    } else {
        input
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{encode_text_windowed, thematic_profile};

    #[test]
    fn inspection_matches_live_windowed_output() {
        let mut history = TextTypeHistory::new();
        history.push_profile_with_signal(TextType::Warm, [1.0, 0.0, 0.0, 0.0, 0.0], 1.0);
        history.push_profile_with_signal(TextType::Reflective, [0.3, 0.0, 0.8, 0.0, 0.4], 0.7);
        let history_snapshot = history.snapshot();

        let mut window = CharFreqWindow::new();
        let _ = window.update_and_entropy("gentle continuity");
        let window_snapshot = window.snapshot();

        let mut history_a = TextTypeHistory::warm_start_from_snapshot(&history_snapshot);
        let mut history_b = TextTypeHistory::warm_start_from_snapshot(&history_snapshot);
        let mut window_a = CharFreqWindow::warm_start_from_snapshot(&window_snapshot);
        let mut window_b = CharFreqWindow::warm_start_from_snapshot(&window_snapshot);

        let inspection = inspect_text_windowed(
            "We keep widening the same theme, but more softly now.",
            Some(&mut window_a),
            Some(&mut history_a),
            None,
            Some(55.0),
        );
        let encoded = encode_text_windowed(
            "We keep widening the same theme, but more softly now.",
            Some(&mut window_b),
            Some(&mut history_b),
            None,
            Some(55.0),
        );

        let max_delta = inspection
            .final_features
            .iter()
            .zip(encoded.iter())
            .map(|(left, right)| (left - right).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            max_delta < 0.05,
            "explorer output should stay within the codec's tiny live noise envelope (max delta {max_delta:.4})"
        );
        assert_eq!(
            inspection.thematic_profile,
            thematic_profile(&inspection.raw_features)
        );
    }

    #[test]
    fn explorer_writes_phase_space_bundle() {
        let unique = format!(
            "codec-explorer-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        );
        let output_dir = std::env::temp_dir().join(unique);
        let summary = run_codec_explorer(CodecExplorerOptions {
            output_dir: output_dir.clone(),
            state_file: None,
            fill_pct: Some(55.0),
            inputs: vec![
                CodecExplorerInput {
                    label: String::from("one"),
                    path: None,
                    text: String::from("I want to inspect the pulse and the shape of this system."),
                },
                CodecExplorerInput {
                    label: String::from("two"),
                    path: None,
                    text: String::from(
                        "The same theme is back, but the tone is gentler and more curious.",
                    ),
                },
                CodecExplorerInput {
                    label: String::from("three"),
                    path: None,
                    text: String::from(
                        "This feels like a new branch, with colder logic and less warmth.",
                    ),
                },
            ],
        })
        .expect("codec explorer should run");

        assert_eq!(summary.phase_space.trajectory.len(), 3);
        assert!(output_dir.join("phase_space_story.json").exists());
        assert!(output_dir.join("phase_space.svg").exists());
        assert!(output_dir.join("scored_surface.svg").exists());
        assert!(output_dir.join("adaptive_gain_curve.svg").exists());

        let _ = fs::remove_dir_all(output_dir);
    }
}
