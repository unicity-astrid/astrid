use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use consciousness_bridge_server::chimera::render;
use consciousness_bridge_server::types::{ChimeraMode, RenderChimeraRequest};

fn unique_temp_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let dir = std::env::temp_dir().join(format!("chimera_render_{name}_{stamp}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_sine_wav(path: &PathBuf, sample_rate: u32, seconds: u32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    let sample_count = sample_rate.saturating_mul(seconds);
    for sample_idx in 0..sample_count {
        let t = (sample_idx as f32) / (sample_rate as f32);
        let sample = ((2.0_f32 * std::f32::consts::PI * 220.0_f32 * t).sin() * 0.25_f32)
            + ((2.0_f32 * std::f32::consts::PI * 330.0_f32 * t).sin() * 0.15_f32);
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
}

fn request(input_path: PathBuf, output_root: PathBuf, mode: ChimeraMode) -> RenderChimeraRequest {
    RenderChimeraRequest {
        input_path,
        mode,
        loops: 1,
        output_root: Some(output_root),
        ..RenderChimeraRequest::default()
    }
}

#[test]
fn golden_path_renders_all_modes() {
    for mode in [
        ChimeraMode::Spectral,
        ChimeraMode::Symbolic,
        ChimeraMode::Dual,
    ] {
        let temp_dir = unique_temp_dir(match mode {
            ChimeraMode::Spectral => "spectral",
            ChimeraMode::Symbolic => "symbolic",
            ChimeraMode::Dual => "dual",
        });
        let input_path = temp_dir.join("input.wav");
        let output_root = temp_dir.join("output");
        write_sine_wav(&input_path, 16_000, 1);

        let result = render(&request(input_path, output_root, mode)).unwrap();
        assert!(result.output_dir.exists());
        assert!(result.manifest_path.exists());
        assert!(!result.emitted_artifacts.is_empty());
        assert_eq!(result.iterations.len(), 1);
        for artifact in &result.emitted_artifacts {
            assert!(
                artifact.path.exists(),
                "missing artifact {}",
                artifact.path.display()
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

#[test]
fn invalid_input_path_fails() {
    let temp_dir = unique_temp_dir("missing_input");
    let output_root = temp_dir.join("output");
    let result = render(&RenderChimeraRequest {
        input_path: temp_dir.join("missing.wav"),
        output_root: Some(output_root),
        ..RenderChimeraRequest::default()
    });
    assert!(result.is_err());
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn unsupported_or_empty_input_fails() {
    let temp_dir = unique_temp_dir("bad_input");

    let bad_path = temp_dir.join("not_audio.txt");
    fs::write(&bad_path, "not a wav").unwrap();
    let bad_result = render(&RenderChimeraRequest {
        input_path: bad_path,
        output_root: Some(temp_dir.join("bad_output")),
        ..RenderChimeraRequest::default()
    });
    assert!(bad_result.is_err());

    let empty_wav = temp_dir.join("empty.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let writer = hound::WavWriter::create(&empty_wav, spec).unwrap();
    writer.finalize().unwrap();
    let empty_result = render(&RenderChimeraRequest {
        input_path: empty_wav,
        output_root: Some(temp_dir.join("empty_output")),
        ..RenderChimeraRequest::default()
    });
    assert!(empty_result.is_err());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn parameter_validation_rejects_out_of_range_values() {
    let temp_dir = unique_temp_dir("params");
    let input_path = temp_dir.join("input.wav");
    write_sine_wav(&input_path, 16_000, 1);

    let too_many_loops = render(&RenderChimeraRequest {
        input_path: input_path.clone(),
        output_root: Some(temp_dir.join("loops_output")),
        loops: 99,
        ..RenderChimeraRequest::default()
    });
    assert!(too_many_loops.is_err());

    let bad_leak = render(&RenderChimeraRequest {
        input_path,
        output_root: Some(temp_dir.join("leak_output")),
        leak: 1.5_f32,
        ..RenderChimeraRequest::default()
    });
    assert!(bad_leak.is_err());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn repeated_renders_are_stable_and_sane() {
    let temp_dir = unique_temp_dir("stability");
    let input_path = temp_dir.join("input.wav");
    write_sine_wav(&input_path, 16_000, 1);

    let result_a = render(&RenderChimeraRequest {
        input_path: input_path.clone(),
        output_root: Some(temp_dir.join("run_a")),
        mode: ChimeraMode::Dual,
        loops: 2,
        ..RenderChimeraRequest::default()
    })
    .unwrap();
    let result_b = render(&RenderChimeraRequest {
        input_path,
        output_root: Some(temp_dir.join("run_b")),
        mode: ChimeraMode::Dual,
        loops: 2,
        ..RenderChimeraRequest::default()
    })
    .unwrap();

    assert_eq!(result_a.iterations.len(), 2);
    assert_eq!(result_b.iterations.len(), 2);

    for (left, right) in result_a.iterations.iter().zip(&result_b.iterations) {
        assert!((0.0_f32..=1.0_f32).contains(&left.blend_symbolic));
        assert!((0.0_f32..=1.0_f32).contains(&right.blend_symbolic));
        assert!(left.n_slow >= left.effective_dims / 10);
        assert!(right.n_slow >= right.effective_dims / 10);
        assert!((left.gap_ratio - right.gap_ratio).abs() < 0.0001_f32);
        assert!((left.blend_symbolic - right.blend_symbolic).abs() < 0.0001_f32);
    }

    let _ = fs::remove_dir_all(&temp_dir);
}
