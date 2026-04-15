//! Native offline spectral chimera renderer.
//!
//! This module keeps the heavy audio/DSP path in the native bridge while
//! exposing a narrow `render(request) -> result` boundary that a future WASM
//! controller can call without owning DSP, audio I/O, or localhost concerns.

use std::f32::consts::PI;
use std::fs;

use anyhow::{Context, Result, anyhow, ensure};
use nalgebra::{DMatrix, DVector, SymmetricEigen};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;

use crate::types::{
    ChimeraIterationMetrics, ChimeraMode, RenderArtifact, RenderChimeraRequest, RenderChimeraResult,
};

#[path = "chimera_support.rs"]
mod support;

use support::{
    add_note, center_matrix, difference_abs, eigengaps, evenly_spaced_indices, matrix_std,
    matrix_variance, median, midi_to_frequency, normalize_audio, quantize_pitch, read_wav_mono,
    resolve_output_dir, row_as_vector, row_norms, row_std, sample_count, select_log_bins,
    select_scale, smooth_columns, validate_request, write_manifest, write_wav_mono,
};

const MAX_LOOPS: u32 = 12;
const N_FFT: usize = 1024;
const HOP_LENGTH: usize = 256;
const ROOT_NOTE: i32 = 60;

/// Run the native chimera render pipeline for an offline WAV input.
pub fn render(request: &RenderChimeraRequest) -> Result<RenderChimeraResult> {
    validate_request(request)?;

    let input_path = fs::canonicalize(&request.input_path).with_context(|| {
        format!(
            "failed to resolve input WAV path {}",
            request.input_path.display()
        )
    })?;
    let output_dir = resolve_output_dir(request)?;
    let loops = usize::try_from(request.loops).context("loop count exceeds usize")?;
    let (mut waveform, sample_rate) = read_wav_mono(&input_path)?;
    ensure!(!waveform.is_empty(), "input WAV has no samples");

    let signal_spec = SignalSpec::new(sample_rate, request.bins)?;
    let reservoir_cfg = ReservoirConfig::from_request(request);
    let mut reservoir = VirtualNodeReservoir::new(&reservoir_cfg)?;
    let mut decomposer = TwinDecomposer::new(reservoir_cfg.effective_dims());
    let mut artifacts = Vec::new();
    let mut iterations = Vec::new();

    for iteration in 0..loops {
        let input_copy = output_dir.join(format!("loop{iteration:02}_input.wav"));
        write_wav_mono(&input_copy, &waveform, sample_rate)?;
        artifacts.push(RenderArtifact {
            kind: "input".to_string(),
            path: input_copy,
        });

        let analysis = analyse_stft(&waveform, &signal_spec)?;
        let states = reservoir.run(&analysis.features);
        let decomposition = decomposer.update(&states)?;

        let spectral_output = if matches!(request.mode, ChimeraMode::Spectral | ChimeraMode::Dual) {
            Some(spectral_path(
                &reservoir,
                &states,
                &analysis,
                request.mix_slow,
                request.mix_fast,
                &signal_spec,
            )?)
        } else {
            None
        };

        if let Some(output) = &spectral_output {
            let slow_path = output_dir.join(format!("loop{iteration:02}_spectral_slow.wav"));
            write_wav_mono(&slow_path, &output.slow_audio, sample_rate)?;
            artifacts.push(RenderArtifact {
                kind: "spectral_slow".to_string(),
                path: slow_path,
            });

            let fast_path = output_dir.join(format!("loop{iteration:02}_spectral_fast.wav"));
            write_wav_mono(&fast_path, &output.fast_audio, sample_rate)?;
            artifacts.push(RenderArtifact {
                kind: "spectral_fast".to_string(),
                path: fast_path,
            });

            let mix_path = output_dir.join(format!("loop{iteration:02}_spectral_mix.wav"));
            write_wav_mono(&mix_path, &output.mix_audio, sample_rate)?;
            artifacts.push(RenderArtifact {
                kind: "spectral_mix".to_string(),
                path: mix_path,
            });
        }

        let symbolic_audio = if matches!(request.mode, ChimeraMode::Symbolic | ChimeraMode::Dual) {
            let symbolic = symbolic_path(
                &decomposition.slow_traj,
                &decomposition.fast_traj,
                &decomposition.scale,
                &signal_spec,
                analysis.original_len,
            );
            let symbolic_path = output_dir.join(format!("loop{iteration:02}_symbolic.wav"));
            write_wav_mono(&symbolic_path, &symbolic, sample_rate)?;
            artifacts.push(RenderArtifact {
                kind: "symbolic".to_string(),
                path: symbolic_path,
            });
            Some(symbolic)
        } else {
            None
        };

        let (next_waveform, blend_symbolic, output_file, output_kind) = match request.mode {
            ChimeraMode::Spectral => {
                let spectral = spectral_output
                    .as_ref()
                    .ok_or_else(|| anyhow!("spectral mode requested but no spectral output"))?;
                let output_file = output_dir.join(format!("loop{iteration:02}_spectral_mix.wav"));
                (
                    spectral.mix_audio.clone(),
                    0.0,
                    Some(output_file),
                    Some("spectral_mix".to_string()),
                )
            },
            ChimeraMode::Symbolic => {
                let symbolic = symbolic_audio
                    .clone()
                    .ok_or_else(|| anyhow!("symbolic mode requested but no symbolic output"))?;
                let output_file = output_dir.join(format!("loop{iteration:02}_symbolic.wav"));
                (
                    symbolic,
                    1.0,
                    Some(output_file),
                    Some("symbolic".to_string()),
                )
            },
            ChimeraMode::Dual => {
                let spectral = spectral_output
                    .as_ref()
                    .ok_or_else(|| anyhow!("dual mode requested but no spectral output"))?;
                let symbolic = symbolic_audio
                    .as_ref()
                    .ok_or_else(|| anyhow!("dual mode requested but no symbolic output"))?;
                let (final_audio, blend_weight) =
                    blend_paths(&spectral.mix_audio, symbolic, decomposition.gap_ratio);
                let output_file = output_dir.join(format!("loop{iteration:02}_final_mix.wav"));
                write_wav_mono(&output_file, &final_audio, sample_rate)?;
                artifacts.push(RenderArtifact {
                    kind: "final_mix".to_string(),
                    path: output_file.clone(),
                });
                (
                    final_audio,
                    blend_weight,
                    Some(output_file),
                    Some("final_mix".to_string()),
                )
            },
        };

        iterations.push(ChimeraIterationMetrics {
            iteration,
            n_slow: decomposition.n_slow,
            gap_ratio: decomposition.gap_ratio,
            aura_variance: decomposition.aura_variance,
            blend_symbolic,
            effective_dims: reservoir_cfg.effective_dims(),
            scale: decomposition.scale.clone(),
            output_file,
        });

        waveform = next_waveform;

        if let Some(kind) = output_kind
            && kind != "final_mix"
            && !matches!(request.mode, ChimeraMode::Dual)
        {
            let final_path = iterations
                .last()
                .and_then(|metrics| metrics.output_file.clone())
                .ok_or_else(|| anyhow!("expected output file for non-dual render"))?;
            artifacts.push(RenderArtifact {
                kind,
                path: final_path,
            });
        }
    }

    let manifest_path = output_dir.join("manifest.json");
    let mut resolved_request = request.clone();
    resolved_request.input_path = input_path;
    resolved_request.output_root = Some(output_dir.clone());
    write_manifest(
        &manifest_path,
        &resolved_request,
        sample_rate,
        &artifacts,
        &iterations,
    )?;

    Ok(RenderChimeraResult {
        output_dir,
        manifest_path,
        mode: request.mode,
        sample_rate,
        emitted_artifacts: artifacts,
        iterations,
    })
}

#[derive(Clone, Copy)]
struct ReservoirConfig {
    physical_nodes: usize,
    virtual_nodes: usize,
    bins: usize,
    leak: f32,
    spectral_radius: f32,
    seed: u64,
}

impl ReservoirConfig {
    fn from_request(request: &RenderChimeraRequest) -> Self {
        Self {
            physical_nodes: request.physical_nodes,
            virtual_nodes: request.virtual_nodes,
            bins: request.bins,
            leak: request.leak,
            spectral_radius: request.spectral_radius,
            seed: request.seed,
        }
    }

    fn effective_dims(self) -> usize {
        self.physical_nodes.saturating_mul(self.virtual_nodes)
    }
}

struct VirtualNodeReservoir {
    cfg: ReservoirConfig,
    w: DMatrix<f32>,
    w_in: DMatrix<f32>,
    w_in_pinv: DMatrix<f32>,
    virtual_mask: DMatrix<f32>,
    state: DVector<f32>,
}

impl VirtualNodeReservoir {
    fn new(cfg: &ReservoirConfig) -> Result<Self> {
        let mut rng = StdRng::seed_from_u64(cfg.seed);
        let mut w = DMatrix::from_fn(cfg.physical_nodes, cfg.physical_nodes, |_, _| {
            if rng.r#gen::<f32>() <= 0.3 {
                rng.gen_range(-0.1_f32..0.1_f32)
            } else {
                0.0
            }
        });

        let singular_values = w.clone().svd(false, false).singular_values;
        let spectral_max = singular_values.iter().copied().fold(0.0_f32, f32::max);
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "float matrix scaling cannot panic"
        )]
        if spectral_max > 0.0 {
            w *= cfg.spectral_radius / spectral_max;
        }

        let w_in = DMatrix::from_fn(cfg.physical_nodes, cfg.bins, |_, _| {
            rng.gen_range(-0.3_f32..0.3_f32)
        });
        let w_in_pinv = w_in
            .clone()
            .svd(true, true)
            .pseudo_inverse(1.0e-6_f32)
            .map_err(|error| {
                anyhow!("failed to compute pseudo-inverse for reservoir input: {error}")
            })?;
        let virtual_mask = DMatrix::from_fn(cfg.physical_nodes, cfg.virtual_nodes, |row, col| {
            let theta = (PI / (cfg.virtual_nodes as f32)) * (col as f32);
            (theta + (row as f32) * 0.5_f32).cos()
        });

        Ok(Self {
            cfg: *cfg,
            w,
            w_in,
            w_in_pinv,
            virtual_mask,
            state: DVector::zeros(cfg.physical_nodes),
        })
    }

    fn run(&mut self, frames: &DMatrix<f32>) -> DMatrix<f32> {
        let mut states = DMatrix::zeros(frames.nrows(), self.cfg.effective_dims());
        for row in 0..frames.nrows() {
            let input = row_as_vector(frames, row);
            let virtual_state = self.step(&input);
            for col in 0..virtual_state.len() {
                states[(row, col)] = virtual_state[col];
            }
        }
        states
    }

    fn decode_spectral(&self, states: &DMatrix<f32>) -> DMatrix<f32> {
        let mut physical_states = DMatrix::zeros(states.nrows(), self.cfg.physical_nodes);
        for row in 0..states.nrows() {
            for node in 0..self.cfg.physical_nodes {
                let mut acc = 0.0_f32;
                for virtual_idx in 0..self.cfg.virtual_nodes {
                    let col = node
                        .checked_mul(self.cfg.virtual_nodes)
                        .and_then(|value| value.checked_add(virtual_idx))
                        .unwrap_or(0);
                    acc += states[(row, col)];
                }
                physical_states[(row, node)] = acc / (self.cfg.virtual_nodes as f32);
            }
        }
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "float matrix multiplication cannot panic"
        )]
        {
            physical_states * self.w_in_pinv.transpose()
        }
    }

    #[expect(
        clippy::arithmetic_side_effects,
        reason = "float matrix/vector arithmetic cannot panic"
    )]
    fn step(&mut self, input: &DVector<f32>) -> DVector<f32> {
        let pre = (&self.w * &self.state) + (&self.w_in * input);
        let updated = pre.map(|value| value.tanh());
        self.state = (&self.state * (1.0_f32 - self.cfg.leak)) + (updated * self.cfg.leak);

        let mut virtual_state = DVector::zeros(self.cfg.effective_dims());
        for node in 0..self.cfg.physical_nodes {
            for virtual_idx in 0..self.cfg.virtual_nodes {
                let out_idx = node
                    .checked_mul(self.cfg.virtual_nodes)
                    .and_then(|value| value.checked_add(virtual_idx))
                    .unwrap_or(0);
                virtual_state[out_idx] = self.state[node] * self.virtual_mask[(node, virtual_idx)];
            }
        }
        virtual_state
    }
}

struct TwinDecomposer {
    cov: DMatrix<f32>,
    momentum: f32,
}

impl TwinDecomposer {
    fn new(dimensions: usize) -> Self {
        Self {
            cov: DMatrix::zeros(dimensions, dimensions),
            momentum: 0.93,
        }
    }

    fn update(&mut self, states: &DMatrix<f32>) -> Result<Decomposition> {
        ensure!(
            states.nrows() > 0,
            "cannot decompose an empty state trajectory"
        );
        ensure!(
            states.ncols() > 0,
            "cannot decompose zero-dimensional state data"
        );

        let centered = center_matrix(states);
        let denom = if states.nrows() > 1 {
            (states.nrows() as f32) - 1.0_f32
        } else {
            1.0_f32
        };
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "float matrix arithmetic cannot panic"
        )]
        let cov_batch = (&centered.transpose() * &centered) * (1.0_f32 / denom);
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "float matrix arithmetic cannot panic"
        )]
        {
            self.cov = (&self.cov * self.momentum) + (cov_batch * (1.0_f32 - self.momentum));
        }

        let eigen = SymmetricEigen::new(self.cov.clone());
        let mut pairs = Vec::with_capacity(states.ncols());
        for idx in 0..states.ncols() {
            pairs.push((
                eigen.eigenvalues[idx],
                eigen.eigenvectors.column(idx).into_owned(),
            ));
        }
        pairs.sort_by(|left, right| right.0.total_cmp(&left.0));

        let evals: Vec<f32> = pairs.iter().map(|(value, _)| *value).collect();
        let gaps = eigengaps(&evals);
        let gap_ratio = if gaps.is_empty() {
            1.0_f32
        } else {
            let largest_gap = gaps.iter().copied().fold(0.0_f32, f32::max);
            let median_gap = median(&gaps).max(1.0e-10_f32);
            largest_gap / median_gap
        };

        let mut n_slow = if gaps.is_empty() {
            states.ncols() / 2
        } else {
            gaps.iter()
                .enumerate()
                .max_by(|left, right| left.1.total_cmp(right.1))
                .map(|(idx, _)| idx.saturating_add(1))
                .unwrap_or(states.ncols() / 2)
        };
        let min_side = states.ncols().max(10) / 10;
        let lo = min_side.max(3).min(states.ncols().saturating_sub(1));
        let hi = states.ncols().saturating_sub(lo);
        n_slow = n_slow.clamp(lo, hi);

        let basis_columns: Vec<DVector<f32>> =
            pairs.into_iter().map(|(_, vector)| vector).collect();
        let slow_basis = DMatrix::from_columns(&basis_columns[..n_slow]);
        let fast_basis = DMatrix::from_columns(&basis_columns[n_slow..]);
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "float matrix multiplication cannot panic"
        )]
        let slow_traj = states * slow_basis;
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "float matrix multiplication cannot panic"
        )]
        let fast_traj = states * fast_basis;
        let aura_variance = matrix_variance(&fast_traj);
        let scale = select_scale(aura_variance).to_string();

        Ok(Decomposition {
            slow_traj,
            fast_traj,
            n_slow,
            gap_ratio,
            aura_variance,
            scale,
        })
    }
}

struct Decomposition {
    slow_traj: DMatrix<f32>,
    fast_traj: DMatrix<f32>,
    n_slow: usize,
    gap_ratio: f32,
    aura_variance: f32,
    scale: String,
}

struct SignalSpec {
    sample_rate: u32,
    n_fft: usize,
    hop_length: usize,
    bins: usize,
    window: Vec<f32>,
    selected_bins: Vec<usize>,
}

impl SignalSpec {
    fn new(sample_rate: u32, bins: usize) -> Result<Self> {
        ensure!(bins >= 4, "bins must be at least 4");
        let mut window = Vec::with_capacity(N_FFT);
        for idx in 0..N_FFT {
            let angle = (2.0_f32 * PI * (idx as f32)) / (N_FFT as f32);
            window.push(0.5_f32 - (0.5_f32 * angle.cos()));
        }
        let selected_bins = select_log_bins(
            N_FFT
                .checked_div(2)
                .and_then(|value| value.checked_add(1))
                .unwrap_or(1),
            bins,
        );
        ensure!(
            !selected_bins.is_empty(),
            "unable to derive any spectral bins from the configured FFT size"
        );
        Ok(Self {
            sample_rate,
            n_fft: N_FFT,
            hop_length: HOP_LENGTH,
            bins,
            window,
            selected_bins,
        })
    }

    fn n_freq(&self) -> usize {
        self.n_fft
            .checked_div(2)
            .and_then(|value| value.checked_add(1))
            .unwrap_or(1)
    }
}

struct StftAnalysis {
    features: DMatrix<f32>,
    phases: Vec<Vec<f32>>,
    original_len: usize,
}

struct SpectralOutput {
    slow_audio: Vec<f32>,
    fast_audio: Vec<f32>,
    mix_audio: Vec<f32>,
}

fn analyse_stft(waveform: &[f32], spec: &SignalSpec) -> Result<StftAnalysis> {
    ensure!(!waveform.is_empty(), "cannot analyse an empty waveform");

    let frame_count = if waveform.len() <= spec.n_fft {
        1
    } else {
        waveform
            .len()
            .saturating_sub(spec.n_fft)
            .checked_add(spec.hop_length.saturating_sub(1))
            .and_then(|value| value.checked_div(spec.hop_length))
            .and_then(|value| value.checked_add(1))
            .unwrap_or(1)
    };
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(spec.n_fft);
    let mut features = DMatrix::zeros(frame_count, spec.bins);
    let mut phases = vec![vec![0.0_f32; spec.n_freq()]; frame_count];

    for frame_idx in 0..frame_count {
        let start = frame_idx.saturating_mul(spec.hop_length);
        let mut buffer = vec![Complex32::new(0.0_f32, 0.0_f32); spec.n_fft];
        for (sample_idx, slot) in buffer.iter_mut().enumerate() {
            let sample = waveform
                .get(start.saturating_add(sample_idx))
                .copied()
                .unwrap_or(0.0);
            slot.re = sample * spec.window[sample_idx];
        }
        fft.process(&mut buffer);

        for bin in 0..spec.n_freq() {
            phases[frame_idx][bin] = buffer[bin].arg();
        }
        for (feature_idx, &bin) in spec.selected_bins.iter().enumerate() {
            if feature_idx >= spec.bins {
                break;
            }
            features[(frame_idx, feature_idx)] = (1.0_f32 + buffer[bin].norm()).ln();
        }
    }

    Ok(StftAnalysis {
        features,
        phases,
        original_len: waveform.len(),
    })
}

fn spectral_path(
    reservoir: &VirtualNodeReservoir,
    states: &DMatrix<f32>,
    analysis: &StftAnalysis,
    mix_slow: f32,
    mix_fast: f32,
    spec: &SignalSpec,
) -> Result<SpectralOutput> {
    let decoded = reservoir.decode_spectral(states);
    let slow_mag = smooth_columns(&decoded, 7);
    let fast_mag = difference_abs(&decoded, &slow_mag);

    let mut slow_audio =
        reconstruct_from_reduced(&slow_mag, &analysis.phases, spec, analysis.original_len)?;
    let mut fast_audio =
        reconstruct_from_reduced(&fast_mag, &analysis.phases, spec, analysis.original_len)?;
    normalize_audio(&mut slow_audio);
    normalize_audio(&mut fast_audio);

    let len = slow_audio.len().min(fast_audio.len());
    let mut mix_audio = Vec::with_capacity(len);
    for idx in 0..len {
        mix_audio.push((mix_slow * slow_audio[idx]) + (mix_fast * fast_audio[idx]));
    }
    normalize_audio(&mut mix_audio);

    Ok(SpectralOutput {
        slow_audio,
        fast_audio,
        mix_audio,
    })
}

fn symbolic_path(
    slow_traj: &DMatrix<f32>,
    fast_traj: &DMatrix<f32>,
    scale: &str,
    spec: &SignalSpec,
    original_len: usize,
) -> Vec<f32> {
    let mut waveform = vec![0.0_f32; original_len];
    if slow_traj.nrows() == 0 || slow_traj.ncols() == 0 {
        return waveform;
    }

    let energy = row_norms(slow_traj);
    let energy_max = energy.iter().copied().fold(0.0_f32, f32::max);
    if energy_max <= 1.0e-8_f32 {
        return waveform;
    }

    let energy_mean = energy.iter().copied().sum::<f32>() / (energy.len() as f32);
    let threshold = energy_mean * 0.8_f32;
    let mut onsets = Vec::new();
    for idx in 1..energy.len() {
        if energy[idx] > threshold && energy[idx.saturating_sub(1)] <= threshold {
            onsets.push(idx);
        }
    }
    if onsets.len() < 4 {
        onsets = evenly_spaced_indices(energy.len(), 12);
    }

    let aura_energy = row_norms(fast_traj);
    let aura_max = aura_energy
        .iter()
        .copied()
        .fold(0.0_f32, f32::max)
        .max(1.0e-8_f32);
    let aura_std = matrix_std(fast_traj).max(1.0e-8_f32);

    for (idx, &frame_idx) in onsets.iter().enumerate() {
        let raw_pitch = (slow_traj[(frame_idx, 0)] * 24.0_f32) + (ROOT_NOTE as f32);
        let midi = quantize_pitch(raw_pitch, ROOT_NOTE, scale);
        let next_idx = onsets
            .get(idx.saturating_add(1))
            .copied()
            .unwrap_or(frame_idx.saturating_add(4));
        let duration_frames = next_idx.saturating_sub(frame_idx).max(1);
        let duration_samples = duration_frames.saturating_mul(spec.hop_length).clamp(
            sample_count(spec.sample_rate, 0.25_f32),
            sample_count(spec.sample_rate, 2.0_f32),
        );
        let start_sample = frame_idx.saturating_mul(spec.hop_length);
        let amplitude = 0.2_f32 + ((aura_energy[frame_idx] / aura_max) * 0.6_f32);
        let local_std = row_std(fast_traj, frame_idx);
        let accent = if local_std > (aura_std * 1.3_f32) {
            1.2_f32
        } else {
            1.0_f32
        };
        add_note(
            &mut waveform,
            start_sample,
            duration_samples,
            midi_to_frequency(midi),
            amplitude * accent,
            spec.sample_rate,
        );
    }

    normalize_audio(&mut waveform);
    waveform
}

fn blend_paths(spectral_audio: &[f32], symbolic_audio: &[f32], gap_ratio: f32) -> (Vec<f32>, f32) {
    let symbolic_weight = 1.0_f32 / (1.0_f32 + (-((gap_ratio - 3.0_f32) / 1.5_f32)).exp());
    let spectral_weight = 1.0_f32 - symbolic_weight;
    let len = spectral_audio.len().min(symbolic_audio.len());
    let mut blended = Vec::with_capacity(len);
    for idx in 0..len {
        blended.push(
            (spectral_weight * spectral_audio[idx]) + (symbolic_weight * symbolic_audio[idx]),
        );
    }
    normalize_audio(&mut blended);
    (blended, symbolic_weight)
}

fn reconstruct_from_reduced(
    reduced: &DMatrix<f32>,
    phases: &[Vec<f32>],
    spec: &SignalSpec,
    original_len: usize,
) -> Result<Vec<f32>> {
    ensure!(
        reduced.nrows() == phases.len(),
        "reduced magnitudes and phase frames disagree on frame count"
    );
    let magnitudes = expand_reduced_magnitudes(reduced, spec);
    let mut frames = Vec::with_capacity(phases.len());
    for frame_idx in 0..phases.len() {
        let mut frame = Vec::with_capacity(spec.n_freq());
        for bin in 0..spec.n_freq() {
            frame.push(Complex32::from_polar(
                magnitudes[(frame_idx, bin)],
                phases[frame_idx][bin],
            ));
        }
        frames.push(frame);
    }
    Ok(istft(&frames, spec, original_len))
}

fn expand_reduced_magnitudes(reduced: &DMatrix<f32>, spec: &SignalSpec) -> DMatrix<f32> {
    let mut expanded = DMatrix::zeros(reduced.nrows(), spec.n_freq());
    let actual_bins = spec.selected_bins.len().min(reduced.ncols());
    if actual_bins == 0 {
        return expanded;
    }

    for row in 0..reduced.nrows() {
        let mut full_frame = vec![0.0_f32; spec.n_freq()];
        for feature_idx in 0..actual_bins {
            let bin = spec.selected_bins[feature_idx];
            full_frame[bin] = reduced[(row, feature_idx)].max(0.0_f32).exp() - 1.0_f32;
        }

        let first_bin = spec.selected_bins[0];
        let first_val = full_frame[first_bin];
        for bin in 0..=first_bin {
            full_frame[bin] = first_val;
        }

        for pair_idx in 0..actual_bins.saturating_sub(1) {
            let start_bin = spec.selected_bins[pair_idx];
            let end_bin = spec.selected_bins[pair_idx.saturating_add(1)];
            let start_val = full_frame[start_bin];
            let end_val = full_frame[end_bin];
            let span = end_bin.saturating_sub(start_bin).max(1);
            for offset in 1..span {
                let alpha = (offset as f32) / (span as f32);
                full_frame[start_bin.saturating_add(offset)] =
                    start_val + ((end_val - start_val) * alpha);
            }
        }

        let last_bin = spec.selected_bins[actual_bins.saturating_sub(1)];
        let last_val = full_frame[last_bin];
        for bin in last_bin..spec.n_freq() {
            full_frame[bin] = last_val;
        }

        for bin in 0..spec.n_freq() {
            expanded[(row, bin)] = full_frame[bin];
        }
    }

    expanded
}

fn istft(frames: &[Vec<Complex32>], spec: &SignalSpec, original_len: usize) -> Vec<f32> {
    if frames.is_empty() {
        return vec![0.0_f32; original_len];
    }

    let out_len = frames
        .len()
        .saturating_sub(1)
        .saturating_mul(spec.hop_length)
        .saturating_add(spec.n_fft);
    let mut output = vec![0.0_f32; out_len];
    let mut norm = vec![0.0_f32; out_len];
    let mut planner = FftPlanner::<f32>::new();
    let ifft = planner.plan_fft_inverse(spec.n_fft);

    for (frame_idx, positive) in frames.iter().enumerate() {
        let start = frame_idx.saturating_mul(spec.hop_length);
        let mut spectrum = vec![Complex32::new(0.0_f32, 0.0_f32); spec.n_fft];
        for (bin, value) in positive.iter().enumerate() {
            if bin < spec.n_freq() {
                spectrum[bin] = *value;
            }
        }
        for bin in 1..spec.n_freq().saturating_sub(1) {
            let mirror = spec.n_fft.saturating_sub(bin);
            spectrum[mirror] = spectrum[bin].conj();
        }
        ifft.process(&mut spectrum);

        for sample_idx in 0..spec.n_fft {
            let pos = start.saturating_add(sample_idx);
            if let Some(out_sample) = output.get_mut(pos) {
                let window = spec.window[sample_idx];
                let value = (spectrum[sample_idx].re / (spec.n_fft as f32)) * window;
                *out_sample += value;
                norm[pos] += window * window;
            }
        }
    }

    for idx in 0..output.len() {
        if norm[idx] > 1.0e-8_f32 {
            output[idx] /= norm[idx];
        }
    }
    output.truncate(original_len);
    output
}
