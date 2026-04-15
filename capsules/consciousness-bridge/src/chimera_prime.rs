//! Prime-scheduled block reservoir for multi-timescale audio processing.
//!
//! Sits between VirtualNodeReservoir and TwinDecomposer in the chimera
//! pipeline. Each block has its own prime period — fast blocks (period=1)
//! track transients, slow blocks (period=7) track mood and timbral drift.
//!
//! The beings see per-block reports showing which temporal layers responded
//! to their audio, giving multi-timescale introspection into sound.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects
)]

use prime_esn_wasm::{BlockSpec, Esn, EsnBuilder};

/// Per-block introspection report after processing.
#[derive(Debug, Clone)]
pub struct BlockReport {
    pub period: usize,
    pub size: usize,
    pub label: &'static str,
    pub energy: f64,
    pub activation_count: usize,
    pub total_frames: usize,
    pub mean_state: f64,
}

/// Full report from prime processing.
#[derive(Debug, Clone)]
pub struct PrimeReport {
    pub blocks: Vec<BlockReport>,
    pub total_frames: usize,
    pub cycle_len: usize,
}

impl PrimeReport {
    /// Format for prompt injection.
    pub fn format_for_prompt(&self, filename: &str) -> String {
        let mut lines = vec![format!("[AUDIO BLOCKS: {filename}]")];
        for b in &self.blocks {
            let active_pct = if self.total_frames > 0 {
                b.activation_count as f64 / self.total_frames as f64 * 100.0
            } else {
                0.0
            };
            lines.push(format!(
                "  Block (period={}, {}): energy={:.3}, active={:.0}%, mean_state={:.4}",
                b.period, b.label, b.energy, active_pct, b.mean_state,
            ));
        }
        lines.push(format!(
            "  Cycle length: {} frames, Total: {} frames",
            self.cycle_len, self.total_frames
        ));
        lines.join("\n")
    }
}

/// Block labels for the five temporal layers.
const BLOCK_LABELS: [&str; 5] = [
    "fast — transients, onsets",
    "rhythm — pulse, meter",
    "contour — articulation, melody",
    "body — harmonic sustain, vowels",
    "mood — timbral drift, memory",
];

/// Build a 5-block prime-scheduled ESN for audio processing.
///
/// Input dimension matches the VirtualNodeReservoir output (typically 64).
/// Output is the full state vector, projected back to input_dim.
pub fn build_audio_esn(input_dim: usize, seed: u64) -> Result<Esn, String> {
    EsnBuilder::new(input_dim)
        .target_radius(0.95)
        .seed(seed)
        .add_block(
            BlockSpec::new(64, 1)
                .leak(1.0)
                .density(0.05)
                .input_scale(0.5),
        )
        .add_block(
            BlockSpec::new(64, 2)
                .leak(0.95)
                .density(0.05)
                .input_scale(0.4),
        )
        .add_block(
            BlockSpec::new(48, 3)
                .leak(0.90)
                .density(0.05)
                .input_scale(0.35),
        )
        .add_block(
            BlockSpec::new(48, 5)
                .leak(0.80)
                .density(0.04)
                .input_scale(0.3),
        )
        .add_block(
            BlockSpec::new(32, 7)
                .leak(0.70)
                .density(0.04)
                .input_scale(0.25),
        )
        .build()
        .map_err(|e| format!("prime ESN build failed: {e}"))
}

/// Process a state trajectory through the prime-scheduled blocks.
///
/// Takes the VirtualNodeReservoir output (F frames × D dims) and runs
/// each frame through the prime ESN. Returns the ESN state trajectory
/// (F frames × total_units) and a per-block introspection report.
///
/// The output is projected back to the input dimension via averaging
/// each block's contribution, keeping it compatible with TwinDecomposer.
pub fn process_trajectory(
    esn: &mut Esn,
    states: &[Vec<f64>],
    input_dim: usize,
) -> (Vec<Vec<f64>>, PrimeReport) {
    esn.reset();

    let schedule = esn.schedule_info();
    let n_frames = states.len();

    // Track per-block activation
    let block_infos: Vec<_> = schedule
        .blocks
        .iter()
        .map(|b| (b.start, b.size, b.period, b.leak))
        .collect();
    let n_blocks = block_infos.len().min(5);
    let mut block_energies = vec![0.0_f64; n_blocks];
    let mut block_activations = vec![0usize; n_blocks];
    let mut block_state_sums = vec![0.0_f64; n_blocks];

    let mut output_trajectory = Vec::with_capacity(n_frames);

    for (frame_idx, frame) in states.iter().enumerate() {
        let full_state = esn.step(frame);

        // Per-block introspection
        for (bi, &(start, size, period, _leak)) in block_infos.iter().enumerate().take(n_blocks) {
            if (frame_idx + 1) % period == 0 {
                block_activations[bi] += 1;
            }
            let block_state = &full_state[start..start + size];
            let energy: f64 = block_state.iter().map(|v| v * v).sum::<f64>() / size as f64;
            block_energies[bi] += energy;
            block_state_sums[bi] += block_state.iter().sum::<f64>() / size as f64;
        }

        // Project back to input_dim by averaging blocks' contributions
        // Each block contributes proportionally to its share of total units
        let mut projected = vec![0.0_f64; input_dim];
        for dim in 0..input_dim {
            let mut sum = 0.0_f64;
            let mut count = 0usize;
            // Sample from each block at proportional positions
            for &(start, size, _period, _leak) in &block_infos {
                let idx = start + (dim * size / input_dim).min(size.saturating_sub(1));
                sum += full_state[idx];
                count += 1;
            }
            if count > 0 {
                projected[dim] = sum / count as f64;
            }
        }
        output_trajectory.push(projected);
    }

    // Normalize block metrics
    let n_f = n_frames.max(1) as f64;
    for bi in 0..n_blocks {
        block_energies[bi] /= n_f;
        block_state_sums[bi] /= n_f;
    }

    let report = PrimeReport {
        blocks: (0..n_blocks)
            .map(|bi| BlockReport {
                period: block_infos[bi].2,
                size: block_infos[bi].1,
                label: BLOCK_LABELS.get(bi).copied().unwrap_or("unknown"),
                energy: block_energies[bi],
                activation_count: block_activations[bi],
                total_frames: n_frames,
                mean_state: block_state_sums[bi],
            })
            .collect(),
        total_frames: n_frames,
        cycle_len: schedule.cycle_len,
    };

    (output_trajectory, report)
}
