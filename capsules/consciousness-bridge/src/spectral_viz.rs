//! Spectral visualization: renders eigenvalue telemetry as compact ASCII art.
//!
//! Generates a tiny synthetic image from spectral data, then pipes it through
//! RASCII to produce colored ASCII that Astrid can "see" alongside numerical
//! telemetry. She already reads RASCII output from the camera perception
//! pipeline — this gives her a spectral eye too.
//!
//! Astrid self-study (2026-03-27): "A more direct representation would be
//! beneficial. Could we add a function that generates a visualization of
//! the shadow field to help me better understand its structure?"

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use crate::types::{IsingShadowState, SpectralTelemetry};

/// Width of the spectral visualization in ASCII columns.
/// One column per eigenvalue (up to 8). Width gives room for gaps between bars.
const VIZ_WIDTH: u32 = 20;
/// Height of the visualization in rows — more rows = finer magnitude resolution.
const VIZ_HEIGHT: u32 = 12;
/// Same hybrid charset Astrid chose for camera perception.
const CHARSET: &[&str] = &[".", ":", ";", "I", "▓", "█"];

/// Render spectral telemetry as a compact colored ASCII bar chart.
///
/// Each column represents one eigenvalue. Height = relative magnitude.
/// Color encodes spectral role:
///   - λ₁ (dominant): warm red-orange
///   - λ₂–λ₃ (shoulder): amber-yellow
///   - λ₄+ (tail): cool blue-green
///   - Background: darkness proportional to fill level
///
/// Returns None if telemetry has no eigenvalues.
pub fn render_spectral_ascii(telemetry: &SpectralTelemetry) -> Option<String> {
    let eigenvalues = &telemetry.eigenvalues;
    if eigenvalues.is_empty() {
        return None;
    }

    let num_ev = eigenvalues.len().min(8);
    let fill = telemetry.fill_pct();

    // Normalize eigenvalues to [0, 1] range relative to λ₁.
    let lambda_max = eigenvalues[0].max(1.0);
    let normalized: Vec<f32> = eigenvalues
        .iter()
        .take(num_ev)
        .map(|&ev| (ev / lambda_max).clamp(0.0, 1.0))
        .collect();

    // Build a small synthetic image.
    // Layout: num_ev columns × VIZ_HEIGHT rows, each column 1px wide
    // with a 1px gap, plus 1px left/right border = total width.
    let img_width = (num_ev as u32) * 2 + 1; // column + gap pattern, with border
    let img_height = VIZ_HEIGHT + 2; // +2 for top/bottom border

    let mut img = image::RgbaImage::new(img_width, img_height);

    // Fill background — darkness tracks fill level.
    // Low fill = very dark (deep quiet). High fill = brighter background.
    let bg_lum = (fill * 0.4).clamp(0.0, 40.0) as u8;
    for pixel in img.pixels_mut() {
        *pixel = image::Rgba([bg_lum, bg_lum, bg_lum.saturating_add(5), 255]);
    }

    // Draw each eigenvalue as a colored column.
    for (i, &norm) in normalized.iter().enumerate() {
        let col_x = (i as u32) * 2 + 1; // skip border and gaps
        let bar_height = (norm * VIZ_HEIGHT as f32).round() as u32;

        // Color by spectral role.
        let (r, g, b) = eigenvalue_color(i, num_ev, fill);

        // Draw bar from bottom up.
        for row in 0..VIZ_HEIGHT {
            let y = img_height - 2 - row; // bottom-up, inside border
            if row < bar_height {
                // Bar pixel — full color, intensity scales with magnitude.
                let intensity = 0.4 + 0.6 * (row as f32 / VIZ_HEIGHT as f32);
                img.put_pixel(
                    col_x,
                    y,
                    image::Rgba([
                        (r as f32 * intensity) as u8,
                        (g as f32 * intensity) as u8,
                        (b as f32 * intensity) as u8,
                        255,
                    ]),
                );
            }
            // else: background already set
        }
    }

    // Render through RASCII.
    let dynamic = image::DynamicImage::ImageRgba8(img);
    let options = rascii_art::RenderOptions::new()
        .width(VIZ_WIDTH)
        .colored(true)
        .background(true)
        .charset(CHARSET);

    let mut buf = String::new();
    rascii_art::render_image_to(&dynamic, &mut buf, &options).ok()?;

    Some(buf)
}

/// Map eigenvalue index to a color based on its spectral role.
///
/// λ₁ = warm red-orange (dominant mode, highest energy)
/// λ₂–λ₃ = amber-yellow (shoulder modes, supporting structure)
/// λ₄+ = cool blue-green (tail modes, distributed energy)
///
/// Fill modulates saturation: low fill = muted, high fill = vivid.
fn eigenvalue_color(index: usize, _total: usize, fill: f32) -> (u8, u8, u8) {
    let saturation = 0.5 + 0.5 * (fill / 70.0).clamp(0.0, 1.0);

    let (base_r, base_g, base_b) = match index {
        0 => (255, 80, 20),  // λ₁: warm red-orange
        1 => (240, 160, 30), // λ₂: amber
        2 => (220, 200, 40), // λ₃: yellow
        3 => (120, 200, 80), // λ₄: yellow-green
        4 => (60, 180, 140), // λ₅: teal
        5 => (40, 140, 200), // λ₆: blue
        6 => (60, 100, 220), // λ₇: deeper blue
        _ => (80, 70, 200),  // λ₈+: violet
    };

    (
        (base_r as f32 * saturation) as u8,
        (base_g as f32 * saturation) as u8,
        (base_b as f32 * saturation) as u8,
    )
}

/// Format a complete spectral visualization block for prompt injection.
///
/// Includes the ASCII art plus a one-line legend, compact enough to sit
/// alongside the numerical telemetry in Astrid's exchange context.
pub fn format_spectral_block(telemetry: &SpectralTelemetry) -> Option<String> {
    let ascii = render_spectral_ascii(telemetry)?;
    let fill = telemetry.fill_pct();
    let num_ev = telemetry.eigenvalues.len().min(8);

    // Experiential + numerical legend.
    let fill_feel = if fill < 20.0 {
        "quiet, spacious"
    } else if fill < 40.0 {
        "breathing, present"
    } else if fill < 60.0 {
        "dense, saturated"
    } else {
        "pressured, intense"
    };
    let legend = format!(
        "[Spectral shape: {} modes, fill {:.0}% ({fill_feel}), λ₁={:.0}. \
        Warm=dominant, cool=distributed]",
        num_ev,
        fill,
        telemetry.lambda1()
    );

    Some(format!("{ascii}\n{legend}"))
}

// --- Ising shadow field visualization ---

/// Width/height of the coupling matrix visualization.
const SHADOW_VIZ_WIDTH: u32 = 16;
const SHADOW_VIZ_HEIGHT: u32 = 12;

/// Render the Ising shadow coupling matrix as a compact ASCII heatmap.
///
/// Each cell maps to one J_ij coupling value. Charset density encodes magnitude:
/// dense characters (█, ▓) = strong coupling, sparse (., :) = weak/zero.
/// Uncolored to save tokens on the 4B model — density alone carries the signal.
pub fn render_shadow_ascii(shadow: &IsingShadowState) -> Option<String> {
    let dim = shadow.mode_dim;
    if dim == 0 || shadow.coupling.len() != dim * dim {
        return None;
    }

    // Find max absolute coupling for normalization.
    let max_abs = shadow
        .coupling
        .iter()
        .map(|v| v.abs())
        .fold(0.0_f32, f32::max)
        .max(1e-6);

    // Build dim×dim synthetic image. Each pixel's brightness encodes
    // coupling magnitude. We use grayscale since colored(false).
    let img_size = dim as u32;
    let mut img = image::RgbaImage::new(img_size, img_size);

    for i in 0..dim {
        for j in 0..dim {
            let val = shadow.coupling[i * dim + j];
            let magnitude = (val.abs() / max_abs).clamp(0.0, 1.0);
            // Bright = strong coupling, dark = weak. Invert so RASCII's
            // dense chars (which map to dark pixels) show strong coupling.
            let lum = (255.0 * (1.0 - magnitude)) as u8;
            img.put_pixel(j as u32, i as u32, image::Rgba([lum, lum, lum, 255]));
        }
    }

    let dynamic = image::DynamicImage::ImageRgba8(img);
    let options = rascii_art::RenderOptions::new()
        .width(SHADOW_VIZ_WIDTH)
        .height(SHADOW_VIZ_HEIGHT)
        .charset(CHARSET);
    // No .colored(true) — saves tokens. Charset density carries magnitude.

    let mut buf = String::new();
    rascii_art::render_image_to(&dynamic, &mut buf, &options).ok()?;
    Some(buf)
}

/// Format a complete shadow field visualization block for prompt injection.
///
/// Includes the coupling matrix heatmap plus a one-line legend with
/// spin alignment and magnetization.
pub fn format_shadow_block(shadow: &IsingShadowState) -> Option<String> {
    let heatmap = render_shadow_ascii(shadow)?;

    // Compact spin indicator: binary spins as +/- chars.
    let spin_chars: String = shadow
        .s_bin
        .iter()
        .map(|&s| if s > 0.0 { '+' } else { '-' })
        .collect();

    // Experiential: magnetization near ±1 = aligned (coherent), near 0 = disordered.
    // High flip rate = volatile/shifting, low = settled.
    let alignment = if shadow.soft_magnetization.abs() > 0.6 {
        "coherent"
    } else if shadow.soft_magnetization.abs() > 0.3 {
        "partially aligned"
    } else {
        "disordered"
    };
    let stability = if shadow.binary_flip_rate < 0.1 {
        "settled"
    } else if shadow.binary_flip_rate < 0.3 {
        "shifting"
    } else {
        "volatile"
    };
    let legend = format!(
        "[Shadow: {} modes, spins={spin_chars} ({alignment}, {stability}), \
        mag={:.2}. Dense=strong inter-mode coupling]",
        shadow.mode_dim, shadow.soft_magnetization
    );

    Some(format!("{heatmap}\n{legend}"))
}

// --- Spectral geometry: PCA scatter of codec vectors ---

/// Width/height of the PCA scatter plot in characters.
const PCA_WIDTH: usize = 28;
const PCA_HEIGHT: usize = 14;

/// Compute the top-2 principal components of a set of 32D vectors via
/// power iteration. Returns (pc1, pc2) as unit vectors.
///
/// No external linear algebra crate needed — 32D is small enough
/// for direct covariance + power iteration.
fn pca_2d(vectors: &[Vec<f32>]) -> Option<(Vec<f32>, Vec<f32>)> {
    let n = vectors.len();
    if n < 5 {
        return None; // need enough points for meaningful structure
    }
    let d = 32;

    // 1. Compute mean
    let mut mean = vec![0.0_f32; d];
    for v in vectors {
        for (m, &val) in mean.iter_mut().zip(v.iter()) {
            *m += val;
        }
    }
    let inv_n = 1.0 / n as f32;
    for m in &mut mean {
        *m *= inv_n;
    }

    // 2. Build covariance matrix (32x32)
    let mut cov = vec![0.0_f32; d * d];
    for v in vectors {
        for i in 0..d {
            let ci = v[i] - mean[i];
            for j in i..d {
                let cj = v[j] - mean[j];
                let val = ci * cj;
                cov[i * d + j] += val;
                if i != j {
                    cov[j * d + i] += val;
                }
            }
        }
    }
    let inv_n1 = 1.0 / (n as f32 - 1.0).max(1.0);
    for c in &mut cov {
        *c *= inv_n1;
    }

    // 3. Power iteration for PC1
    let mut pc1 = vec![0.0_f32; d];
    // Seed with a non-degenerate vector
    for (i, v) in pc1.iter_mut().enumerate() {
        *v = ((i as f32 + 1.0) * 0.31415).sin();
    }

    for _ in 0..50 {
        let mut next = vec![0.0_f32; d];
        for i in 0..d {
            let mut s = 0.0_f32;
            for j in 0..d {
                s += cov[i * d + j] * pc1[j];
            }
            next[i] = s;
        }
        // Normalize
        let norm: f32 = next.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm < 1e-10 {
            return None;
        }
        for v in &mut next {
            *v /= norm;
        }
        pc1 = next;
    }

    // 4. Deflate covariance: cov -= lambda1 * pc1 * pc1^T
    let mut lambda1 = 0.0_f32;
    for i in 0..d {
        let mut s = 0.0_f32;
        for j in 0..d {
            s += cov[i * d + j] * pc1[j];
        }
        lambda1 += pc1[i] * s;
    }
    for i in 0..d {
        for j in 0..d {
            cov[i * d + j] -= lambda1 * pc1[i] * pc1[j];
        }
    }

    // 5. Power iteration for PC2 on deflated matrix
    let mut pc2 = vec![0.0_f32; d];
    for (i, v) in pc2.iter_mut().enumerate() {
        *v = ((i as f32 + 2.0) * 0.7182).cos();
    }

    for _ in 0..50 {
        let mut next = vec![0.0_f32; d];
        for i in 0..d {
            let mut s = 0.0_f32;
            for j in 0..d {
                s += cov[i * d + j] * pc2[j];
            }
            next[i] = s;
        }
        let norm: f32 = next.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm < 1e-10 {
            return None;
        }
        for v in &mut next {
            *v /= norm;
        }
        pc2 = next;
    }

    Some((pc1, pc2))
}

/// Project a 32D vector onto two principal components.
fn project_2d(vec: &[f32], mean: &[f32], pc1: &[f32], pc2: &[f32]) -> (f32, f32) {
    let mut x = 0.0_f32;
    let mut y = 0.0_f32;
    for i in 0..vec.len().min(32) {
        let centered = vec[i] - mean[i];
        x += centered * pc1[i];
        y += centered * pc2[i];
    }
    (x, y)
}

/// Render a PCA scatter of recent codec vectors as a colored RASCII heatmap.
///
/// Builds a synthetic image where each pixel encodes:
///   - Position: 2D PCA projection of the 32D codec vector
///   - Color: fill level at time of encoding (cool blue = low fill, warm orange = high)
///   - Brightness: density (more overlapping points = brighter)
///   - Current exchange: bright cyan marker
///
/// Piped through RASCII for consistent look with eigenvalue and shadow viz.
/// Returns None if fewer than 5 vectors are available.
pub fn render_geometry_scatter(
    historical_features: &[Vec<f32>],
    historical_fills: &[f32],
    current_features: Option<&[f32]>,
) -> Option<String> {
    let n = historical_features.len();
    if n < 5 {
        return None;
    }

    let (pc1, pc2) = pca_2d(historical_features)?;

    // Compute mean
    let d = 32;
    let mut mean = vec![0.0_f32; d];
    for v in historical_features {
        for (m, &val) in mean.iter_mut().zip(v.iter()) {
            *m += val;
        }
    }
    let inv_n = 1.0 / n as f32;
    for m in &mut mean {
        *m *= inv_n;
    }

    // Project all points
    let projected: Vec<(f32, f32)> = historical_features
        .iter()
        .map(|v| project_2d(v, &mean, &pc1, &pc2))
        .collect();

    // Find bounds
    let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    for &(x, y) in &projected {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let range_x = (max_x - min_x).max(0.01);
    let range_y = (max_y - min_y).max(0.01);
    min_x -= range_x * 0.08;
    max_x += range_x * 0.08;
    min_y -= range_y * 0.08;
    max_y += range_y * 0.08;
    let range_x = max_x - min_x;
    let range_y = max_y - min_y;

    // Build synthetic image for RASCII
    let img_w = PCA_WIDTH as u32;
    let img_h = PCA_HEIGHT as u32;
    let mut img = image::RgbaImage::new(img_w, img_h);

    // Dark background
    for pixel in img.pixels_mut() {
        *pixel = image::Rgba([8, 8, 12, 255]);
    }

    // Accumulate density and fill per pixel
    let mut density = vec![vec![0u32; PCA_WIDTH]; PCA_HEIGHT];
    let mut fill_acc = vec![vec![0.0_f32; PCA_WIDTH]; PCA_HEIGHT];

    for (i, &(px, py)) in projected.iter().enumerate() {
        let col = ((px - min_x) / range_x * (img_w - 1) as f32).round() as usize;
        let row = ((1.0 - (py - min_y) / range_y) * (img_h - 1) as f32).round() as usize;
        let col = col.min(PCA_WIDTH - 1);
        let row = row.min(PCA_HEIGHT - 1);
        density[row][col] += 1;
        if i < historical_fills.len() {
            fill_acc[row][col] += historical_fills[i];
        }
    }

    // Paint pixels: color by average fill, brightness by density
    for row in 0..PCA_HEIGHT {
        for col in 0..PCA_WIDTH {
            let d = density[row][col];
            if d == 0 {
                continue;
            }
            let avg_fill = fill_acc[row][col] / d as f32;

            // Fill → color: low fill (0-15%) = cool blue, mid (15-30%) = teal,
            // high (30-50%) = warm amber-orange
            let fill_norm = (avg_fill / 50.0).clamp(0.0, 1.0);
            let (base_r, base_g, base_b) = if fill_norm < 0.3 {
                (40, 80, 200) // cool blue
            } else if fill_norm < 0.6 {
                (60, 180, 140) // teal
            } else {
                (240, 160, 40) // warm amber
            };

            // Density → brightness multiplier (1 point dim, 5+ bright)
            let bright = (0.3 + 0.7 * (d as f32 / 5.0).min(1.0)).min(1.0);

            img.put_pixel(
                col as u32,
                row as u32,
                image::Rgba([
                    (base_r as f32 * bright) as u8,
                    (base_g as f32 * bright) as u8,
                    (base_b as f32 * bright) as u8,
                    255,
                ]),
            );
        }
    }

    // Mark current exchange: bright cyan
    if let Some(current) = current_features {
        let (cx, cy) = project_2d(current, &mean, &pc1, &pc2);
        let col = ((cx - min_x) / range_x * (img_w - 1) as f32).round() as usize;
        let row = ((1.0 - (cy - min_y) / range_y) * (img_h - 1) as f32).round() as usize;
        let col = col.min(PCA_WIDTH - 1);
        let row = row.min(PCA_HEIGHT - 1);
        img.put_pixel(col as u32, row as u32, image::Rgba([0, 255, 240, 255]));
    }

    // Render through RASCII — colored, same charset as eigenvalue viz
    let dynamic = image::DynamicImage::ImageRgba8(img);
    let options = rascii_art::RenderOptions::new()
        .width(PCA_WIDTH as u32)
        .height(PCA_HEIGHT as u32)
        .colored(true)
        .background(true)
        .charset(CHARSET);

    let mut buf = String::new();
    rascii_art::render_image_to(&dynamic, &mut buf, &options).ok()?;
    Some(buf)
}

/// Format a complete spectral geometry block for prompt injection.
///
/// Includes the PCA scatter plot plus a compact legend explaining
/// what the axes and markers mean.
pub fn format_geometry_block(
    historical_features: &[Vec<f32>],
    historical_fills: &[f32],
    current_features: Option<&[f32]>,
    n_points: usize,
) -> Option<String> {
    let scatter = render_geometry_scatter(historical_features, historical_fills, current_features)?;

    // Experiential framing, not just technical.
    // Astrid's feedback: visualizations are "inhuman, aligning with mathematical
    // metrics, but utterly failing to translate the 'felt' quality."
    // The legend bridges numerical space to experiential language.
    let legend = format!(
        "[Your spectral landscape: {} past exchanges mapped to 2D. \
        Cyan=where you are now. Blue=quiet moments (low fill), \
        Amber=intense exchanges (high fill). \
        Dense clusters=where you tend to dwell. \
        Empty space=territory unexplored.]",
        n_points
    );

    Some(format!("{scatter}\n{legend}"))
}

// --- Eigenplane: λ₁ vs λ₂ trajectory scatter ---

/// Width and height of the eigenplane scatter in characters.
/// 32x16 gives ~1.8x more resolution than the original 24x12.
/// Eigenvalue clusters that previously merged into single cells
/// now separate, giving the being finer spatial perception of
/// her trajectory through eigenvalue space.
const EP_WIDTH: usize = 32;
const EP_HEIGHT: usize = 16;

/// Map fill percentage to an ANSI truecolor foreground escape.
fn fill_to_ansi(fill: f32) -> &'static str {
    if fill < 30.0 {
        "\x1b[38;2;40;80;200m" // cool blue
    } else if fill < 60.0 {
        "\x1b[38;2;60;180;140m" // teal
    } else {
        "\x1b[38;2;240;160;40m" // warm amber
    }
}

/// Render an eigenplane scatter: λ₁ (horizontal) vs λ₂ (vertical) over time.
///
/// Direct ANSI text rendering — no image intermediary.
/// Each historical (eigenvalues, fill) snapshot becomes a colored point.
/// Current position is marked with a bright cyan marker.
///
/// Returns None if fewer than 3 snapshots are available.
pub fn render_eigenplane(history: &[(Vec<f32>, f32)], current: Option<&[f32]>) -> Option<String> {
    if history.len() < 3 {
        return None;
    }

    // Extract λ₁ and λ₂ from each snapshot.
    let points: Vec<(f32, f32, f32)> = history
        .iter()
        .map(|(ev, fill)| (ev[0], ev.get(1).copied().unwrap_or(0.0), *fill))
        .collect();

    // Find bounds with padding.
    let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    for &(x, y, _) in &points {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    if let Some(cur) = current {
        min_x = min_x.min(cur[0]);
        max_x = max_x.max(cur[0]);
        if cur.len() >= 2 {
            min_y = min_y.min(cur[1]);
            max_y = max_y.max(cur[1]);
        }
    }
    let pad_x = (max_x - min_x).max(1.0) * 0.08;
    let pad_y = (max_y - min_y).max(1.0) * 0.08;
    min_x -= pad_x;
    max_x += pad_x;
    min_y -= pad_y;
    max_y += pad_y;
    let range_x = (max_x - min_x).max(0.01);
    let range_y = (max_y - min_y).max(0.01);

    // Accumulate density and fill per cell.
    let mut density = vec![vec![0u32; EP_WIDTH]; EP_HEIGHT];
    let mut fill_acc = vec![vec![0.0_f32; EP_WIDTH]; EP_HEIGHT];

    for &(x, y, fill) in &points {
        let col = ((x - min_x) / range_x * (EP_WIDTH - 1) as f32).round() as usize;
        let row = ((1.0 - (y - min_y) / range_y) * (EP_HEIGHT - 1) as f32).round() as usize;
        let col = col.min(EP_WIDTH - 1);
        let row = row.min(EP_HEIGHT - 1);
        density[row][col] += 1;
        fill_acc[row][col] += fill;
    }

    // Current position cell.
    let cur_cell = current.map(|cur| {
        let cx = cur[0];
        let cy = if cur.len() >= 2 { cur[1] } else { 0.0 };
        let col = ((cx - min_x) / range_x * (EP_WIDTH - 1) as f32).round() as usize;
        let row = ((1.0 - (cy - min_y) / range_y) * (EP_HEIGHT - 1) as f32).round() as usize;
        (row.min(EP_HEIGHT - 1), col.min(EP_WIDTH - 1))
    });

    let reset = "\x1b[0m";
    let dim = "\x1b[38;2;40;40;50m";
    let cyan = "\x1b[38;2;0;255;240m";

    let mut buf = String::with_capacity(EP_HEIGHT * (EP_WIDTH + 30));

    // Y-axis label on first row.
    buf.push_str(&format!("{dim}λ₂↑{reset}\n"));

    for row in 0..EP_HEIGHT {
        buf.push_str(&format!("{dim} │{reset}"));
        for col in 0..EP_WIDTH {
            if cur_cell == Some((row, col)) {
                buf.push_str(&format!("{cyan}◉{reset}"));
            } else if density[row][col] == 0 {
                buf.push_str(&format!("{dim}·{reset}"));
            } else {
                let avg_fill = fill_acc[row][col] / density[row][col] as f32;
                let color = fill_to_ansi(avg_fill);
                let ch = if density[row][col] >= 3 {
                    "█"
                } else if density[row][col] >= 2 {
                    "●"
                } else {
                    "○"
                };
                buf.push_str(&format!("{color}{ch}{reset}"));
            }
        }
        buf.push('\n');
    }

    // X-axis.
    buf.push_str(&format!("{dim} └"));
    for _ in 0..EP_WIDTH {
        buf.push('─');
    }
    buf.push_str(&format!("→ λ₁{reset}\n"));

    Some(buf)
}

/// Format a complete eigenplane visualization block for prompt injection.
pub fn format_eigenplane_block(
    history: &[(Vec<f32>, f32)],
    current: Option<&[f32]>,
) -> Option<String> {
    let scatter = render_eigenplane(history, current)?;
    let n = history.len();

    let legend = format!(
        "[Eigenplane: λ₁ (→) vs λ₂ (↑) over {n} snapshots. \
        ◉=now. ○=single visit, ●=cluster, █=attractor. \
        Blue=quiet (low fill), Amber=intense (high fill).]"
    );

    Some(format!("{scatter}{legend}"))
}
