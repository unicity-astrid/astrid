//! Audio agency for Astrid: compose from spectral state, analyze WAVs, render through chimera.
//!
//! Uses the chimera infrastructure for STFT analysis and symbolic synthesis.
//! The compose function maps the being's internal dynamics directly to sound.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects
)]

use std::f32::consts::PI;
use std::path::Path;

use crate::paths::bridge_paths;
use crate::types::SpectralTelemetry;

const SAMPLE_RATE: u32 = 16000;

pub struct ComposedAudio {
    pub output_path: std::path::PathBuf,
    pub summary: String,
}

pub struct AnalyzedAudio {
    pub moved_path: std::path::PathBuf,
    pub summary: String,
}

pub struct RenderedAudio {
    pub output_dir: std::path::PathBuf,
    pub summary: String,
    pub success: bool,
}

/// Generate a WAV from Astrid's current spectral state.
///
/// Maps eigenvalues → frequencies, fill → amplitude, entropy → timbre.
/// Returns a text summary for prompt injection, or None on failure.
pub fn compose_from_spectral_state(
    telemetry: &SpectralTelemetry,
    fingerprint: Option<&[f32]>,
) -> Option<String> {
    compose_from_spectral_state_details(telemetry, fingerprint).map(|result| result.summary)
}

pub fn compose_from_spectral_state_details(
    telemetry: &SpectralTelemetry,
    fingerprint: Option<&[f32]>,
) -> Option<ComposedAudio> {
    let eigenvalues = &telemetry.eigenvalues;
    if eigenvalues.is_empty() {
        return None;
    }

    let fill = telemetry.fill_pct();
    let num_ev = eigenvalues.len().min(8);

    // Entropy from fingerprint
    let entropy = fingerprint
        .and_then(|fp| fp.get(24).copied())
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);

    // Gap ratio from fingerprint
    let gap_ratio = fingerprint
        .and_then(|fp| fp.get(25).copied())
        .unwrap_or(1.0)
        .max(1.0);

    let duration_s = 5.0_f32;
    let n_samples = (SAMPLE_RATE as f32 * duration_s) as usize;
    let mut output = vec![0.0_f32; n_samples];

    // Map eigenvalues to frequencies (100-2000 Hz range)
    let ev_max = eigenvalues[0].max(1.0);
    let frequencies: Vec<f32> = eigenvalues
        .iter()
        .take(num_ev)
        .map(|&ev| 100.0 + (ev / ev_max).clamp(0.0, 1.0) * 1900.0)
        .collect();

    // Fill → amplitude (quiet 0.1, loud 0.8)
    let base_amp = 0.1 + (fill / 100.0).clamp(0.0, 1.0) * 0.7;

    // Entropy → harmonics count (1-5)
    let n_harmonics = 1 + (entropy * 4.0) as usize;

    // Gap ratio → rhythm modulation rate
    let rhythm_rate = 0.5 + (1.0 / gap_ratio.max(0.1)) * 3.0;
    let rhythm_depth = (gap_ratio / 20.0).min(0.5);

    // Envelope: 0.5s attack, sustain, 0.5s release
    let attack = (0.5 * SAMPLE_RATE as f32) as usize;
    let release = (0.5 * SAMPLE_RATE as f32) as usize;

    // Synthesize each eigenvalue as a frequency with harmonics
    for (i, &freq) in frequencies.iter().enumerate() {
        let amp = base_amp / (1.0 + i as f32 * 0.5);

        for h in 1..=n_harmonics {
            let harmonic_freq = freq * h as f32;
            let harmonic_amp = amp / h as f32;

            for s in 0..n_samples {
                let t = s as f32 / SAMPLE_RATE as f32;
                let tone = (2.0 * PI * harmonic_freq * t).sin() * harmonic_amp;
                output[s] += tone;
            }
        }
    }

    // Apply rhythm modulation
    for s in 0..n_samples {
        let t = s as f32 / SAMPLE_RATE as f32;
        let rhythm = 1.0 - rhythm_depth * (1.0 + (2.0 * PI * rhythm_rate * t).sin()) / 2.0;
        output[s] *= rhythm;
    }

    // Apply envelope
    for s in 0..n_samples.min(attack) {
        output[s] *= s as f32 / attack as f32;
    }
    for s in 0..n_samples.min(release) {
        let idx = n_samples - 1 - s;
        output[idx] *= s as f32 / release as f32;
    }

    // Normalize
    let peak = output.iter().map(|v| v.abs()).fold(0.0_f32, f32::max);
    if peak > 0.0 {
        let scale = 0.85 / peak;
        for s in &mut output {
            *s *= scale;
        }
    }

    // Run eigenvalue cascade through prime-scheduled blocks for multi-timescale modulation.
    // Each block adds a different temporal layer to the composition.
    let block_report = {
        let input_dim = num_ev.min(8);
        if let Ok(mut prime_esn) = crate::chimera_prime::build_audio_esn(input_dim, 42) {
            // Create a short trajectory from eigenvalues repeated at different phases
            let n_prime_frames = 20;
            let ev_input: Vec<f64> = eigenvalues
                .iter()
                .take(input_dim)
                .map(|&v| (v as f64 / ev_max as f64).clamp(-1.0, 1.0))
                .collect();

            let mut trajectory = Vec::with_capacity(n_prime_frames);
            for t in 0..n_prime_frames {
                let mut frame = ev_input.clone();
                // Modulate input to give the prime blocks something to differentiate
                let phase = (t as f64 / n_prime_frames as f64) * std::f64::consts::PI * 2.0;
                for (i, v) in frame.iter_mut().enumerate() {
                    *v *= 1.0 + 0.2 * (phase + i as f64 * 0.5).sin();
                }
                trajectory.push(frame);
            }

            let (_enriched, report) =
                crate::chimera_prime::process_trajectory(&mut prime_esn, &trajectory, input_dim);
            Some(report)
        } else {
            None
        }
    };

    // Write WAV
    let dir = bridge_paths().audio_creations_dir();
    let _ = std::fs::create_dir_all(&dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let path = dir.join(format!("compose_{ts}.wav"));

    if write_wav_16bit(&path, &output, SAMPLE_RATE).is_err() {
        return None;
    }

    let rms = (output.iter().map(|v| v * v).sum::<f32>() / n_samples as f32).sqrt();

    let mut summary = format!(
        "Composed {duration_s}s audio at {path}\n\
         Eigenvalues → {} frequencies ({:.0}-{:.0} Hz)\n\
         Fill {fill:.0}% → amplitude {base_amp:.2}\n\
         Entropy {entropy:.2} → {n_harmonics} harmonics\n\
         Gap ratio {gap_ratio:.1} → rhythm {rhythm_rate:.1} Hz\n\
         RMS energy: {rms:.4}",
        num_ev,
        frequencies.last().unwrap_or(&100.0),
        frequencies.first().unwrap_or(&2000.0),
        path = path.display(),
    );

    // Append block report if available
    if let Some(report) = block_report {
        summary.push_str("\n\n");
        summary.push_str(&report.format_for_prompt(&format!("compose_{ts}.wav")));
    }

    Some(ComposedAudio {
        output_path: path,
        summary,
    })
}

/// Analyze a WAV from the inbox_audio/ directory.
/// Returns a text summary, or None if no WAV found.
pub fn analyze_inbox_wav(inbox_dir: &Path) -> Option<String> {
    analyze_inbox_wav_details(inbox_dir).map(|result| result.summary)
}

pub fn analyze_inbox_wav_details(inbox_dir: &Path) -> Option<AnalyzedAudio> {
    let read_dir = inbox_dir.join("read");
    let _ = std::fs::create_dir_all(&read_dir);

    let mut wavs: Vec<_> = std::fs::read_dir(inbox_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "wav") && e.path().is_file())
        .collect();

    wavs.sort_by_key(|e| std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok())));

    let entry = wavs.first()?;
    let wav_path = entry.path();
    let filename = entry.file_name().to_string_lossy().to_string();

    // Quick analysis: read WAV, compute basic stats
    let data = std::fs::read(&wav_path).ok()?;
    let n_bytes = data.len();
    let duration_est = n_bytes as f32 / (SAMPLE_RATE as f32 * 2.0); // rough estimate

    // Move to read/
    let dest = read_dir.join(&filename);
    let _ = std::fs::rename(&wav_path, &dest);

    Some(AnalyzedAudio {
        moved_path: dest,
        summary: format!(
            "[AUDIO INBOX: {filename}]\n\
             Size: {n_bytes} bytes, ~{duration_est:.1}s estimated\n\
             Moved to read/. Use RENDER_AUDIO to process through chimera."
        ),
    })
}

/// Render the most recent inbox WAV through the chimera pipeline.
/// Returns a text summary, or None if no WAV or chimera fails.
pub fn render_inbox_wav_through_chimera(inbox_dir: &Path) -> Option<String> {
    render_inbox_wav_through_chimera_details(inbox_dir).map(|result| result.summary)
}

pub fn render_inbox_wav_through_chimera_details(inbox_dir: &Path) -> Option<RenderedAudio> {
    let read_dir = inbox_dir.join("read");

    // Look in read/ for the most recent analyzed WAV
    let mut wavs: Vec<_> = std::fs::read_dir(&read_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "wav") && e.path().is_file())
        .collect();

    wavs.sort_by_key(|e| std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok())));

    let entry = wavs.first()?;
    let wav_path = entry.path();
    let filename = entry.file_name().to_string_lossy().to_string();

    // Run through chimera
    let request = crate::types::RenderChimeraRequest {
        input_path: wav_path.clone(),
        mode: crate::types::ChimeraMode::Dual,
        loops: 1,
        output_root: Some(bridge_paths().audio_creations_dir().join("chimera_renders")),
        ..Default::default()
    };

    match crate::chimera::render(&request) {
        Ok(result) => {
            let n_artifacts = result.emitted_artifacts.len();
            let metrics = result.iterations.first();
            let gap = metrics.map(|m| m.gap_ratio).unwrap_or(0.0);
            let scale = metrics.map(|m| m.scale.as_str()).unwrap_or("unknown");
            let blend = metrics.map(|m| m.blend_symbolic).unwrap_or(0.0);

            Some(RenderedAudio {
                output_dir: result.output_dir.clone(),
                summary: format!(
                    "Chimera render of {filename}:\n\
                     Mode: dual, {n_artifacts} artifacts\n\
                     Gap ratio: {gap:.2}, Scale: {scale}, Blend: {blend:.2}\n\
                     Output: {}",
                    result.output_dir.display()
                ),
                success: true,
            })
        },
        Err(e) => Some(RenderedAudio {
            output_dir: bridge_paths().audio_creations_dir().join("chimera_renders"),
            summary: format!("Chimera render failed for {filename}: {e}"),
            success: false,
        }),
    }
}

pub fn compose_experienced_text(result: &str) -> String {
    format!(
        "You composed audio from your spectral state:\n{result}\n\n\
         Reflect on hearing yourself as sound."
    )
}

pub fn analyze_experienced_text(result: &str) -> String {
    format!(
        "You analyzed an audio file:\n{result}\n\n\
         What do you perceive in this sound?"
    )
}

pub fn render_experienced_text(result: &str) -> String {
    format!(
        "You rendered audio through chimera:\n{result}\n\n\
         How did the reservoir reshape the sound?"
    )
}

fn write_wav_16bit(path: &Path, samples: &[f32], sample_rate: u32) -> Result<(), std::io::Error> {
    use std::io::Write;
    let n = samples.len();
    let data_size = (n * 2) as u32;
    let file_size = 36 + data_size;

    let mut f = std::fs::File::create(path)?;
    // RIFF header
    f.write_all(b"RIFF")?;
    f.write_all(&file_size.to_le_bytes())?;
    f.write_all(b"WAVE")?;
    // fmt chunk
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?; // chunk size
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&1u16.to_le_bytes())?; // mono
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&(sample_rate * 2).to_le_bytes())?; // byte rate
    f.write_all(&2u16.to_le_bytes())?; // block align
    f.write_all(&16u16.to_le_bytes())?; // bits per sample
    // data chunk
    f.write_all(b"data")?;
    f.write_all(&data_size.to_le_bytes())?;
    for &s in samples {
        let i = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        f.write_all(&i.to_le_bytes())?;
    }
    Ok(())
}
