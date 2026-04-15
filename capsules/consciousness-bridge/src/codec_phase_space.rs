use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::codec::THEMATIC_DIMS;
use crate::codec_explorer::CodecExplorerEntryReport;

const THEME_LABELS: [&str; THEMATIC_DIMS] =
    ["inquiry", "certainty", "warmth", "tension", "curiosity"];

#[derive(Debug, Clone, Serialize)]
pub struct CodecExplorerPhasePoint {
    pub index: usize,
    pub label: String,
    pub segment: String,
    pub pc1: f32,
    pub pc2: f32,
    pub continuous_thematic_relevance: f32,
    pub discrete_recurrence_contribution: f32,
    pub novelty_divergence_moderation: f32,
    pub final_modulation: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodecExplorerPhaseSpace {
    pub basis: String,
    pub axes: Vec<String>,
    pub explained_variance: [f32; 2],
    pub trajectory: Vec<CodecExplorerPhasePoint>,
    pub segment_sizes: [usize; 3],
}

pub fn build_phase_space_report(entries: &[CodecExplorerEntryReport]) -> CodecExplorerPhaseSpace {
    let profiles = entries
        .iter()
        .map(|entry| entry.thematic_profile)
        .collect::<Vec<_>>();
    let (mean, basis, explained) = fit_thematic_pca(&profiles);
    let trajectory = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let centered = subtract_profile(&entry.thematic_profile, &mean);
            let pc1 = dot_profile(&centered, &basis[0]);
            let pc2 = dot_profile(&centered, &basis[1]);
            CodecExplorerPhasePoint {
                index,
                label: entry.label.clone(),
                segment: phase_segment(index, entries.len()).to_string(),
                pc1,
                pc2,
                continuous_thematic_relevance: entry.resonance.continuous_thematic_relevance,
                discrete_recurrence_contribution: entry.resonance.discrete_recurrence_contribution,
                novelty_divergence_moderation: entry.resonance.novelty_divergence_moderation,
                final_modulation: entry.resonance.final_modulation,
            }
        })
        .collect::<Vec<_>>();
    let segment_sizes = [
        trajectory
            .iter()
            .filter(|point| point.segment == "early")
            .count(),
        trajectory
            .iter()
            .filter(|point| point.segment == "middle")
            .count(),
        trajectory
            .iter()
            .filter(|point| point.segment == "late")
            .count(),
    ];
    CodecExplorerPhaseSpace {
        basis: String::from("thematic_profile_pca"),
        axes: THEME_LABELS
            .iter()
            .map(|label| (*label).to_string())
            .collect(),
        explained_variance: explained,
        trajectory,
        segment_sizes,
    }
}

pub fn write_phase_space_story(
    output_dir: &Path,
    phase_space: &CodecExplorerPhaseSpace,
) -> Result<()> {
    let path = output_dir.join("phase_space_story.json");
    fs::write(&path, serde_json::to_string_pretty(phase_space)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

pub fn write_phase_space_svg(
    output_dir: &Path,
    phase_space: &CodecExplorerPhaseSpace,
) -> Result<()> {
    let path = output_dir.join("phase_space.svg");
    let width = 940.0_f32;
    let height = 380.0_f32;
    let left = 70.0_f32;
    let top = 50.0_f32;
    let panel_w = 380.0_f32;
    let panel_h = 260.0_f32;
    let gap = 70.0_f32;
    let points = &phase_space.trajectory;
    let xs = points.iter().map(|point| point.pc1).collect::<Vec<_>>();
    let ys = points.iter().map(|point| point.pc2).collect::<Vec<_>>();
    let min_x = xs.iter().cloned().fold(0.0_f32, f32::min);
    let max_x = xs.iter().cloned().fold(0.0_f32, f32::max);
    let min_y = ys.iter().cloned().fold(0.0_f32, f32::min);
    let max_y = ys.iter().cloned().fold(0.0_f32, f32::max);
    let scale_x = (max_x - min_x).max(1e-6);
    let scale_y = (max_y - min_y).max(1e-6);
    let map_point = |pc1: f32, pc2: f32, x_offset: f32| -> (f32, f32) {
        let x = x_offset + ((pc1 - min_x) / scale_x) * panel_w;
        let y = top + panel_h - ((pc2 - min_y) / scale_y) * panel_h;
        (x, y)
    };

    let mut svg = String::new();
    svg.push_str(&svg_header(width, height));
    svg.push_str(r#"<text x="24" y="28" font-size="18" font-family="monospace">Astrid Codec Explorer: thematic phase space</text>"#);
    svg.push_str(&format!(
        r#"<text x="24" y="48" font-size="11" font-family="monospace">Shared PCA over thematic profiles | PC1={:.3} PC2={:.3}</text>"#,
        phase_space.explained_variance[0],
        phase_space.explained_variance[1]
    ));
    for (panel_idx, title) in ["shared trajectory", "early / middle / late"]
        .iter()
        .enumerate()
    {
        let x = left + panel_idx as f32 * (panel_w + gap);
        svg.push_str(&format!(
            r##"<rect x="{x:.1}" y="{top:.1}" width="{panel_w:.1}" height="{panel_h:.1}" fill="none" stroke="#d1d5db" stroke-width="1"/>"##
        ));
        svg.push_str(&format!(
            r#"<text x="{:.1}" y="{:.1}" font-size="13" font-family="monospace">{}</text>"#,
            x,
            top - 10.0,
            title
        ));
    }

    let shared_points = points
        .iter()
        .map(|point| {
            let (x, y) = map_point(point.pc1, point.pc2, left);
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    svg.push_str(&format!(
        r##"<polyline fill="none" stroke="#64748b" stroke-width="2" points="{shared_points}"/>"##
    ));
    for point in points {
        let (x, y) = map_point(point.pc1, point.pc2, left);
        svg.push_str(&format!(
            r##"<circle cx="{x:.1}" cy="{y:.1}" r="4" fill="#2563eb" opacity="0.85"/><text x="{:.1}" y="{:.1}" font-size="9" font-family="monospace">{}</text>"##,
            x + 6.0,
            y - 6.0,
            xml_escape(&point.label)
        ));
    }

    let segment_colors = [
        ("early", "#2563eb"),
        ("middle", "#16a34a"),
        ("late", "#dc2626"),
    ];
    for (segment, color) in segment_colors {
        let segment_points = points
            .iter()
            .filter(|point| point.segment == segment)
            .map(|point| {
                let (x, y) = map_point(point.pc1, point.pc2, left + panel_w + gap);
                format!("{x:.1},{y:.1}")
            })
            .collect::<Vec<_>>();
        if segment_points.is_empty() {
            continue;
        }
        svg.push_str(&format!(
            r##"<polyline fill="none" stroke="{color}" stroke-width="3" points="{}"/>"##,
            segment_points.join(" ")
        ));
        let legend_x = left
            + panel_w
            + gap
            + 12.0
            + match segment {
                "early" => 0.0,
                "middle" => 120.0,
                _ => 240.0,
            };
        svg.push_str(&format!(
            r##"<rect x="{legend_x:.1}" y="{:.1}" width="14" height="14" fill="{color}"/><text x="{:.1}" y="{:.1}" font-size="11" font-family="monospace">{segment}</text>"##,
            top + panel_h + 20.0,
            legend_x + 20.0,
            top + panel_h + 31.0,
        ));
    }
    svg.push_str("</svg>\n");
    fs::write(&path, svg).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn fit_thematic_pca(
    profiles: &[[f32; THEMATIC_DIMS]],
) -> ([f32; THEMATIC_DIMS], [[f32; THEMATIC_DIMS]; 2], [f32; 2]) {
    let mut mean = [0.0_f32; THEMATIC_DIMS];
    if profiles.is_empty() {
        return (
            mean,
            [[1.0, 0.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0, 0.0]],
            [0.0, 0.0],
        );
    }
    for profile in profiles {
        for (idx, value) in profile.iter().enumerate() {
            mean[idx] += *value;
        }
    }
    let denom = profiles.len() as f32;
    for value in &mut mean {
        *value /= denom;
    }
    if profiles.len() < 2 {
        return (
            mean,
            [[1.0, 0.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0, 0.0]],
            [1.0, 0.0],
        );
    }

    let mut covariance = [[0.0_f32; THEMATIC_DIMS]; THEMATIC_DIMS];
    for profile in profiles {
        let centered = subtract_profile(profile, &mean);
        for row in 0..THEMATIC_DIMS {
            for col in 0..THEMATIC_DIMS {
                covariance[row][col] += centered[row] * centered[col];
            }
        }
    }
    let cov_denom = (profiles.len().saturating_sub(1)) as f32;
    if cov_denom > 0.0 {
        for row in &mut covariance {
            for value in row {
                *value /= cov_denom;
            }
        }
    }

    let (pc1, lambda1) = power_iteration(covariance, [1.0, 0.0, 0.0, 0.0, 0.0]);
    let mut deflated = covariance;
    for row in 0..THEMATIC_DIMS {
        for col in 0..THEMATIC_DIMS {
            deflated[row][col] -= lambda1 * pc1[row] * pc1[col];
        }
    }
    let (pc2, lambda2) = power_iteration(deflated, [0.0, 1.0, 0.0, 0.0, 0.0]);
    let total_variance = covariance
        .iter()
        .enumerate()
        .map(|(idx, row)| row[idx].max(0.0))
        .sum::<f32>();
    let explained = if total_variance > f32::EPSILON {
        [
            (lambda1.max(0.0) / total_variance).clamp(0.0, 1.0),
            (lambda2.max(0.0) / total_variance).clamp(0.0, 1.0),
        ]
    } else {
        [0.0, 0.0]
    };
    (mean, [pc1, pc2], explained)
}

fn subtract_profile(
    profile: &[f32; THEMATIC_DIMS],
    mean: &[f32; THEMATIC_DIMS],
) -> [f32; THEMATIC_DIMS] {
    let mut output = [0.0_f32; THEMATIC_DIMS];
    for idx in 0..THEMATIC_DIMS {
        output[idx] = profile[idx] - mean[idx];
    }
    output
}

fn dot_profile(left: &[f32; THEMATIC_DIMS], right: &[f32; THEMATIC_DIMS]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(l, r)| l * r)
        .sum::<f32>()
}

fn mat_vec_mul(
    matrix: &[[f32; THEMATIC_DIMS]; THEMATIC_DIMS],
    vector: &[f32; THEMATIC_DIMS],
) -> [f32; THEMATIC_DIMS] {
    let mut output = [0.0_f32; THEMATIC_DIMS];
    for row in 0..THEMATIC_DIMS {
        output[row] = dot_profile(&matrix[row], vector);
    }
    output
}

fn normalize_vector(vector: [f32; THEMATIC_DIMS]) -> [f32; THEMATIC_DIMS] {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return [1.0, 0.0, 0.0, 0.0, 0.0];
    }
    let mut output = [0.0_f32; THEMATIC_DIMS];
    for idx in 0..THEMATIC_DIMS {
        output[idx] = vector[idx] / norm;
    }
    output
}

fn power_iteration(
    matrix: [[f32; THEMATIC_DIMS]; THEMATIC_DIMS],
    seed: [f32; THEMATIC_DIMS],
) -> ([f32; THEMATIC_DIMS], f32) {
    let mut vector = normalize_vector(seed);
    for _ in 0..24 {
        let next = mat_vec_mul(&matrix, &vector);
        vector = normalize_vector(next);
    }
    let mv = mat_vec_mul(&matrix, &vector);
    let eigenvalue = dot_profile(&vector, &mv);
    (vector, eigenvalue)
}

fn phase_segment(index: usize, total: usize) -> &'static str {
    if total <= 1 {
        return "early";
    }
    let left = total / 3;
    let right = (2 * total) / 3;
    if index < left {
        "early"
    } else if index < right {
        "middle"
    } else {
        "late"
    }
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
