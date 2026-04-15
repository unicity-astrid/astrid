use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::codec::{DEFAULT_SEMANTIC_GAIN, NAMED_CODEC_DIMS, SEMANTIC_DIM, adaptive_gain};
use crate::codec_explorer::{CodecExplorerEntryReport, CodecExplorerSummary};

pub fn write_scored_surface_bundle(summary: &CodecExplorerSummary) -> Result<()> {
    write_scored_surface_svg(&summary.output_dir, &summary.entries)?;
    write_adaptive_gain_curve_svg(&summary.output_dir, summary.fill_pct)?;
    Ok(())
}

fn write_scored_surface_svg(output_dir: &Path, entries: &[CodecExplorerEntryReport]) -> Result<()> {
    let path = output_dir.join("scored_surface.svg");
    let width = 1260.0_f32;
    let height = (160.0 + entries.len() as f32 * 30.0).max(260.0);
    let left = 180.0_f32;
    let top = 60.0_f32;
    let cell_w = 16.0_f32;
    let cell_h = 20.0_f32;
    let score_left = left + SEMANTIC_DIM as f32 * cell_w + 28.0;
    let score_bar_w = 110.0_f32;
    let mut svg = String::new();
    svg.push_str(&svg_header(width, height));
    svg.push_str(
        r#"<text x="24" y="28" font-size="20" font-family="monospace">Astrid Codec Explorer: scored semantic surface</text>"#,
    );
    svg.push_str(
        r##"<text x="24" y="48" font-size="11" font-family="monospace" fill="#4b5563">48D features are shown as a heat surface; the right-side bars show how continuous relevance, discrete recurrence, novelty moderation, and final gain shape the score.</text>"##,
    );

    let max_abs = entries
        .iter()
        .flat_map(|entry| entry.feature_vector.iter())
        .map(|value| value.abs())
        .fold(1.0_f32, f32::max);

    for dim in 0..SEMANTIC_DIM {
        let x = left + dim as f32 * cell_w;
        if dim % 4 == 0 {
            svg.push_str(&format!(
                r##"<text x="{x:.1}" y="{:.1}" font-size="9" font-family="monospace" transform="rotate(60 {x:.1} {:.1})">f{dim:02}</text>"##,
                top - 10.0,
                top - 10.0
            ));
        }
    }
    for (label, idx) in NAMED_CODEC_DIMS {
        let x = left + idx as f32 * cell_w;
        svg.push_str(&format!(
            r##"<line x1="{x:.1}" y1="{:.1}" x2="{x:.1}" y2="{:.1}" stroke="#6b7280" stroke-dasharray="2 3" stroke-width="1"/>"##,
            top - 2.0,
            top + entries.len() as f32 * cell_h + 6.0
        ));
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="{:.1}" font-size="9" font-family="monospace" fill="#374151">{}</text>"##,
            x + 2.0,
            height - 18.0,
            xml_escape(label)
        ));
    }

    let score_labels = [
        ("cont", "#2563eb"),
        ("disc", "#16a34a"),
        ("nov", "#f59e0b"),
        ("gain", "#dc2626"),
    ];
    for (index, (label, color)) in score_labels.iter().enumerate() {
        let x = score_left + index as f32 * (score_bar_w + 10.0);
        svg.push_str(&format!(
            r##"<text x="{x:.1}" y="{:.1}" font-size="10" font-family="monospace" fill="{color}">{label}</text>"##,
            top - 12.0
        ));
    }

    for (row_index, entry) in entries.iter().enumerate() {
        let y = top + row_index as f32 * cell_h;
        svg.push_str(&format!(
            r#"<text x="18" y="{:.1}" font-size="10" font-family="monospace">{}</text>"#,
            y + cell_h * 0.72,
            xml_escape(&entry.label)
        ));
        for (dim, value) in entry.feature_vector.iter().enumerate() {
            let x = left + dim as f32 * cell_w;
            svg.push_str(&format!(
                r##"<rect x="{x:.1}" y="{y:.1}" width="{:.1}" height="{:.1}" fill="{}" stroke="#111827" stroke-opacity="0.08"/>"##,
                cell_w - 1.0,
                cell_h - 1.0,
                diverging_color(*value, max_abs)
            ));
        }
        let score_values = [
            entry.resonance.continuous_thematic_relevance,
            entry.resonance.discrete_recurrence_contribution,
            entry.resonance.novelty_divergence_moderation,
            entry.resonance.final_effective_gain / DEFAULT_SEMANTIC_GAIN.max(f32::EPSILON),
        ];
        for (score_index, value) in score_values.iter().enumerate() {
            let x = score_left + score_index as f32 * (score_bar_w + 10.0);
            let normalized = (*value / 1.5).clamp(0.0, 1.0);
            let fill_w = score_bar_w * normalized;
            svg.push_str(&format!(
                r##"<rect x="{x:.1}" y="{:.1}" width="{:.1}" height="10" fill="#e5e7eb"/>"##,
                y + 4.0,
                score_bar_w
            ));
            svg.push_str(&format!(
                r##"<rect x="{x:.1}" y="{:.1}" width="{fill_w:.1}" height="10" fill="{}"/>"##,
                y + 4.0,
                score_labels[score_index].1
            ));
            svg.push_str(&format!(
                r#"<text x="{:.1}" y="{:.1}" font-size="9" font-family="monospace">{:.2}</text>"#,
                x + score_bar_w + 4.0,
                y + 12.0,
                value
            ));
        }
    }

    svg.push_str("</svg>\n");
    fs::write(&path, svg).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn write_adaptive_gain_curve_svg(output_dir: &Path, fill_pct: Option<f32>) -> Result<()> {
    let path = output_dir.join("adaptive_gain_curve.svg");
    let width = 960.0_f32;
    let height = 320.0_f32;
    let left = 80.0_f32;
    let top = 40.0_f32;
    let plot_w = width - left - 28.0;
    let plot_h = height - top - 56.0;
    let max_gain = (0..=100)
        .map(|fill| adaptive_gain(Some(fill as f32)))
        .fold(DEFAULT_SEMANTIC_GAIN, f32::max)
        .max(DEFAULT_SEMANTIC_GAIN);
    let mut points = String::new();
    for fill in 0..=100 {
        let gain = adaptive_gain(Some(fill as f32));
        let x = left + plot_w * (fill as f32 / 100.0);
        let y = top + plot_h - plot_h * (gain / max_gain.max(f32::EPSILON));
        if !points.is_empty() {
            points.push(' ');
        }
        points.push_str(&format!("{x:.1},{y:.1}"));
    }
    let baseline_y = top + plot_h - plot_h * (DEFAULT_SEMANTIC_GAIN / max_gain.max(f32::EPSILON));

    let mut svg = String::new();
    svg.push_str(&svg_header(width, height));
    svg.push_str(
        r#"<text x="24" y="28" font-size="20" font-family="monospace">Astrid Codec Explorer: adaptive gain curve</text>"#,
    );
    svg.push_str(
        r##"<text x="24" y="48" font-size="11" font-family="monospace" fill="#4b5563">This is the live codec gain curve: quiet at low fill, fuller near the operational middle, and capped at the default semantic gain ceiling.</text>"##,
    );
    svg.push_str(&format!(
        r##"<line x1="{left}" y1="{baseline_y:.1}" x2="{:.1}" y2="{baseline_y:.1}" stroke="#9ca3af" stroke-dasharray="4 4"/>"##,
        left + plot_w
    ));
    svg.push_str(&format!(
        r##"<text x="{:.1}" y="{:.1}" font-size="10" font-family="monospace" fill="#6b7280">DEFAULT_SEMANTIC_GAIN {:.2}</text>"##,
        left + 8.0,
        baseline_y - 6.0,
        DEFAULT_SEMANTIC_GAIN
    ));
    svg.push_str(&format!(
        r##"<polyline fill="none" stroke="#2563eb" stroke-width="3" points="{points}"/>"##
    ));
    for tick in 0..=5 {
        let frac = tick as f32 / 5.0;
        let y = top + plot_h - plot_h * frac;
        let value = max_gain * frac;
        svg.push_str(&format!(
            r##"<line x1="{left}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" stroke="#e5e7eb" stroke-width="1"/>"##,
            left + plot_w
        ));
        svg.push_str(&format!(
            r#"<text x="16" y="{:.1}" font-size="10" font-family="monospace">{value:.2}</text>"#,
            y + 3.0
        ));
    }
    for tick in (0..=100).step_by(20) {
        let x = left + plot_w * (tick as f32 / 100.0);
        svg.push_str(&format!(
            r##"<line x1="{x:.1}" y1="{top}" x2="{x:.1}" y2="{:.1}" stroke="#f3f4f6" stroke-width="1"/>"##,
            top + plot_h
        ));
        svg.push_str(&format!(
            r#"<text x="{:.1}" y="{:.1}" font-size="10" font-family="monospace">{tick}</text>"#,
            x - 8.0,
            height - 16.0
        ));
    }
    if let Some(fill) = fill_pct {
        let x = left + plot_w * (fill.clamp(0.0, 100.0) / 100.0);
        let gain = adaptive_gain(Some(fill));
        let y = top + plot_h - plot_h * (gain / max_gain.max(f32::EPSILON));
        svg.push_str(&format!(
            r##"<circle cx="{x:.1}" cy="{y:.1}" r="5" fill="#dc2626"/>"##
        ));
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="{:.1}" font-size="10" font-family="monospace" fill="#991b1b">fill {:.1}% → gain {:.2}</text>"##,
            x + 8.0,
            y - 8.0,
            fill,
            gain
        ));
    }
    svg.push_str("</svg>\n");
    fs::write(&path, svg).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
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

fn diverging_color(value: f32, max_abs: f32) -> String {
    let scale = if max_abs > f32::EPSILON {
        (value / max_abs).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    if scale >= 0.0 {
        let intensity = (scale * 180.0) as u8;
        format!(
            "#{:02x}{:02x}{:02x}",
            255_u8,
            255_u8.saturating_sub(intensity),
            255_u8.saturating_sub(intensity)
        )
    } else {
        let intensity = ((-scale) * 180.0) as u8;
        format!(
            "#{:02x}{:02x}{:02x}",
            255_u8.saturating_sub(intensity),
            255_u8.saturating_sub(intensity),
            255_u8
        )
    }
}
