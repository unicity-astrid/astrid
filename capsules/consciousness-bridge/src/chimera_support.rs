use std::f32::consts::PI;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, ensure};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use nalgebra::{DMatrix, DVector};
use serde::Serialize;

use crate::types::{ChimeraIterationMetrics, RenderArtifact, RenderChimeraRequest};

use super::MAX_LOOPS;

pub(super) fn add_note(
    waveform: &mut [f32],
    start_sample: usize,
    duration_samples: usize,
    frequency: f32,
    amplitude: f32,
    sample_rate: u32,
) {
    if start_sample >= waveform.len() {
        return;
    }

    let max_len = waveform.len().saturating_sub(start_sample);
    let duration = duration_samples.min(max_len);
    let attack = sample_count(sample_rate, 0.02_f32).min(duration.max(1));
    let release = sample_count(sample_rate, 0.08_f32).min(duration.max(1));
    let sustain_start = attack.min(duration);
    let release_start = duration.saturating_sub(release);

    for idx in 0..duration {
        let env = if idx < sustain_start {
            (idx as f32) / (attack.max(1) as f32)
        } else if idx >= release_start {
            let remaining = duration.saturating_sub(idx);
            (remaining as f32) / (release.max(1) as f32)
        } else {
            1.0_f32
        };
        let time = (idx as f32) / (sample_rate as f32);
        let phase = 2.0_f32 * PI * frequency * time;
        let tone = phase.sin()
            + (0.35_f32 * (2.0_f32 * phase).sin())
            + (0.2_f32 * (3.0_f32 * phase).sin());
        waveform[start_sample.saturating_add(idx)] += tone * env * amplitude * 0.25_f32;
    }
}

pub(super) fn validate_request(request: &RenderChimeraRequest) -> Result<()> {
    ensure!(
        !request.input_path.as_os_str().is_empty(),
        "input_path is required for chimera renders"
    );
    ensure!(request.loops > 0, "loops must be at least 1");
    ensure!(request.loops <= MAX_LOOPS, "loops must be <= {MAX_LOOPS}");
    ensure!(
        request.physical_nodes >= 4 && request.physical_nodes <= 64,
        "physical_nodes must be between 4 and 64"
    );
    ensure!(
        request.virtual_nodes >= 2 && request.virtual_nodes <= 32,
        "virtual_nodes must be between 2 and 32"
    );
    ensure!(
        request.bins >= 4 && request.bins <= 128,
        "bins must be between 4 and 128"
    );
    ensure!(
        (0.0_f32..=1.0_f32).contains(&request.leak) && request.leak > 0.0_f32,
        "leak must be within (0, 1]"
    );
    ensure!(
        request.spectral_radius > 0.0_f32 && request.spectral_radius <= 2.0_f32,
        "spectral_radius must be within (0, 2]"
    );
    ensure!(
        request.mix_slow >= 0.0_f32 && request.mix_fast >= 0.0_f32,
        "mix weights must be non-negative"
    );
    ensure!(
        (request.mix_slow + request.mix_fast) > 0.0_f32,
        "mix_slow + mix_fast must be > 0"
    );
    Ok(())
}

pub(super) fn resolve_output_dir(request: &RenderChimeraRequest) -> Result<PathBuf> {
    let base_root = match &request.output_root {
        Some(path) => absolutize(path)?,
        None => absolutize(&PathBuf::from("workspace/chimera"))?,
    };
    let output_dir = if request.output_root.is_some() {
        base_root
    } else {
        base_root.join(format!("render_{}", unix_timestamp_millis()?))
    };
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create output directory {}", output_dir.display()))?;
    fs::canonicalize(&output_dir).or(Ok(output_dir))
}

fn absolutize(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("failed to read current working directory")?
            .join(path))
    }
}

fn unix_timestamp_millis() -> Result<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_millis())
}

pub(super) fn read_wav_mono(path: &Path) -> Result<(Vec<f32>, u32)> {
    let mut reader =
        WavReader::open(path).with_context(|| format!("failed to open WAV {}", path.display()))?;
    let spec = reader.spec();
    ensure!(spec.channels > 0, "WAV must have at least one channel");

    let interleaved = match spec.sample_format {
        SampleFormat::Float => read_float_samples(&mut reader)?,
        SampleFormat::Int => read_int_samples(&mut reader, spec.bits_per_sample)?,
    };
    ensure!(
        !interleaved.is_empty(),
        "input WAV has no decodable samples"
    );

    let channels = usize::from(spec.channels);
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "channels.max(1) ensures non-zero divisor"
    )]
    let mono_capacity = interleaved.len() / channels.max(1);
    let mut mono = Vec::with_capacity(mono_capacity);
    for frame in interleaved.chunks(channels) {
        let sum = frame.iter().copied().sum::<f32>();
        mono.push(sum / (frame.len() as f32));
    }
    Ok((mono, spec.sample_rate))
}

fn read_float_samples(reader: &mut WavReader<std::io::BufReader<File>>) -> Result<Vec<f32>> {
    let mut samples = Vec::new();
    for sample in reader.samples::<f32>() {
        samples.push(sample.context("failed to decode float WAV sample")?);
    }
    Ok(samples)
}

fn read_int_samples(
    reader: &mut WavReader<std::io::BufReader<File>>,
    bits_per_sample: u16,
) -> Result<Vec<f32>> {
    ensure!(
        (1_u16..=32_u16).contains(&bits_per_sample),
        "unsupported integer WAV depth: {bits_per_sample}"
    );
    let shift = u32::from(bits_per_sample.saturating_sub(1));
    let scale = (1_i64)
        .checked_shl(shift)
        .ok_or_else(|| anyhow!("invalid integer WAV depth: {bits_per_sample}"))
        .and_then(|v| {
            v.checked_sub(1_i64)
                .ok_or_else(|| anyhow!("WAV scale underflow"))
        })? as f32;
    let mut samples = Vec::new();
    for sample in reader.samples::<i32>() {
        let value = sample.context("failed to decode integer WAV sample")?;
        samples.push((value as f32) / scale.max(1.0_f32));
    }
    Ok(samples)
}

pub(super) fn write_wav_mono(path: &Path, waveform: &[f32], sample_rate: u32) -> Result<()> {
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(path, spec)
        .with_context(|| format!("failed to create WAV {}", path.display()))?;
    for sample in waveform {
        writer
            .write_sample(*sample)
            .with_context(|| format!("failed writing sample to {}", path.display()))?;
    }
    writer
        .finalize()
        .with_context(|| format!("failed to finalize WAV {}", path.display()))?;
    Ok(())
}

pub(super) fn write_manifest(
    path: &Path,
    request: &RenderChimeraRequest,
    sample_rate: u32,
    artifacts: &[RenderArtifact],
    iterations: &[ChimeraIterationMetrics],
) -> Result<()> {
    #[derive(Serialize)]
    struct Manifest<'a> {
        request: &'a RenderChimeraRequest,
        sample_rate: u32,
        emitted_artifacts: &'a [RenderArtifact],
        iterations: &'a [ChimeraIterationMetrics],
    }

    let manifest = Manifest {
        request,
        sample_rate,
        emitted_artifacts: artifacts,
        iterations,
    };
    let file =
        File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
    serde_json::to_writer_pretty(file, &manifest)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    Ok(())
}

pub(super) fn select_log_bins(n_freq: usize, bins: usize) -> Vec<usize> {
    if n_freq <= 2 || bins == 0 {
        return vec![];
    }

    let mut selected = Vec::new();
    let end = (n_freq.saturating_sub(1)) as f32;
    for idx in 0..bins {
        let t = if bins == 1 {
            0.0_f32
        } else {
            (idx as f32) / ((bins.saturating_sub(1)) as f32)
        };
        let raw = (end.ln() * t).exp().round();
        let bounded = raw.clamp(1.0_f32, end) as usize;
        if selected.last().copied() != Some(bounded) {
            selected.push(bounded);
        }
    }
    selected
}

pub(super) fn row_as_vector(matrix: &DMatrix<f32>, row: usize) -> DVector<f32> {
    DVector::from_iterator(matrix.ncols(), matrix.row(row).iter().copied())
}

pub(super) fn center_matrix(matrix: &DMatrix<f32>) -> DMatrix<f32> {
    let mut centered = matrix.clone();
    for col in 0..matrix.ncols() {
        let mut mean = 0.0_f32;
        for row in 0..matrix.nrows() {
            mean += matrix[(row, col)];
        }
        mean /= matrix.nrows().max(1) as f32;
        for row in 0..matrix.nrows() {
            centered[(row, col)] -= mean;
        }
    }
    centered
}

pub(super) fn eigengaps(evals: &[f32]) -> Vec<f32> {
    let mut gaps = Vec::new();
    for idx in 0..evals.len().saturating_sub(1) {
        gaps.push(evals[idx].abs() - evals[idx.saturating_add(1)].abs());
    }
    gaps
}

pub(super) fn median(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 1.0_f32;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid.saturating_sub(1)] + sorted[mid]) * 0.5_f32
    } else {
        sorted[mid]
    }
}

pub(super) fn smooth_columns(matrix: &DMatrix<f32>, kernel_size: usize) -> DMatrix<f32> {
    let radius = kernel_size / 2;
    let mut smoothed = DMatrix::zeros(matrix.nrows(), matrix.ncols());
    for col in 0..matrix.ncols() {
        for row in 0..matrix.nrows() {
            let start = row.saturating_sub(radius);
            let end = row
                .saturating_add(radius)
                .min(matrix.nrows().saturating_sub(1));
            let mut acc = 0.0_f32;
            let mut count = 0_usize;
            for sample_idx in start..=end {
                acc += matrix[(sample_idx, col)];
                count = count.saturating_add(1);
            }
            smoothed[(row, col)] = (acc / (count.max(1) as f32)).max(0.0_f32);
        }
    }
    smoothed
}

pub(super) fn difference_abs(left: &DMatrix<f32>, right: &DMatrix<f32>) -> DMatrix<f32> {
    let mut diff = DMatrix::zeros(left.nrows(), left.ncols());
    for row in 0..left.nrows() {
        for col in 0..left.ncols() {
            diff[(row, col)] = (left[(row, col)] - right[(row, col)]).abs();
        }
    }
    diff
}

pub(super) fn row_norms(matrix: &DMatrix<f32>) -> Vec<f32> {
    let mut norms = Vec::with_capacity(matrix.nrows());
    for row in 0..matrix.nrows() {
        let mut acc = 0.0_f32;
        for col in 0..matrix.ncols() {
            let value = matrix[(row, col)];
            acc += value * value;
        }
        norms.push(acc.sqrt());
    }
    norms
}

pub(super) fn matrix_variance(matrix: &DMatrix<f32>) -> f32 {
    if matrix.is_empty() {
        return 0.0_f32;
    }
    let count = matrix.nrows().saturating_mul(matrix.ncols()).max(1);
    let mean = matrix.iter().copied().sum::<f32>() / (count as f32);
    matrix
        .iter()
        .map(|value| {
            let delta = *value - mean;
            delta * delta
        })
        .sum::<f32>()
        / (count as f32)
}

pub(super) fn matrix_std(matrix: &DMatrix<f32>) -> f32 {
    matrix_variance(matrix).sqrt()
}

pub(super) fn row_std(matrix: &DMatrix<f32>, row: usize) -> f32 {
    if matrix.ncols() == 0 {
        return 0.0_f32;
    }
    let mean = matrix.row(row).iter().copied().sum::<f32>() / (matrix.ncols() as f32);
    let variance = matrix
        .row(row)
        .iter()
        .map(|value| {
            let delta = *value - mean;
            delta * delta
        })
        .sum::<f32>()
        / (matrix.ncols() as f32);
    variance.sqrt()
}

pub(super) fn evenly_spaced_indices(len: usize, desired: usize) -> Vec<usize> {
    if len == 0 || desired == 0 {
        return vec![];
    }
    if desired >= len {
        return (0..len).collect();
    }

    let mut indices = Vec::with_capacity(desired);
    let last = len.saturating_sub(1);
    let denom = desired.saturating_sub(1).max(1);
    for idx in 0..desired {
        let pos = idx.saturating_mul(last).checked_div(denom).unwrap_or(0);
        if indices.last().copied() != Some(pos) {
            indices.push(pos);
        }
    }
    indices
}

pub(super) fn select_scale(aura_variance: f32) -> &'static str {
    if aura_variance < 0.3_f32 {
        "pentatonic"
    } else if aura_variance < 0.6_f32 {
        "dorian"
    } else if aura_variance < 0.8_f32 {
        "aeolian"
    } else if aura_variance < 1.2_f32 {
        "whole_tone"
    } else {
        "chromatic"
    }
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "MIDI pitch arithmetic on small bounded integers cannot overflow"
)]
pub(super) fn quantize_pitch(raw_midi: f32, root: i32, scale: &str) -> i32 {
    let intervals: &[i32] = match scale {
        "pentatonic" => &[0, 2, 4, 7, 9],
        "dorian" => &[0, 2, 3, 5, 7, 9, 10],
        "aeolian" => &[0, 2, 3, 5, 7, 8, 10],
        "whole_tone" => &[0, 2, 4, 6, 8, 10],
        _ => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
    };

    let midi = raw_midi.round() as i32;
    let octave = (midi - root).div_euclid(12);
    let pitch_class = (midi - root).rem_euclid(12);
    let mut nearest = intervals[0];
    let mut distance = i32::MAX;
    for interval in intervals {
        let candidate_distance = (pitch_class - *interval).abs();
        if candidate_distance < distance {
            distance = candidate_distance;
            nearest = *interval;
        }
    }
    (root + (octave * 12) + nearest).clamp(36, 96)
}

pub(super) fn midi_to_frequency(midi: i32) -> f32 {
    440.0_f32 * (2.0_f32).powf(((midi as f32) - 69.0_f32) / 12.0_f32)
}

pub(super) fn sample_count(sample_rate: u32, seconds: f32) -> usize {
    ((sample_rate as f32) * seconds).round().max(1.0_f32) as usize
}

pub(super) fn normalize_audio(waveform: &mut [f32]) {
    let peak = waveform
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max);
    if peak > 0.0_f32 {
        let scale = peak * 1.05_f32;
        for sample in waveform {
            *sample /= scale;
        }
    }
}
