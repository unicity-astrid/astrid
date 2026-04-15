use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;
use clap::Parser;
use rascii_art::RenderOptions;
use serde::Serialize;

#[path = "../../shared/managed_dir.rs"]
mod managed_dir;

/// Astrid reported width 20 as "almost too detailed... a little exhausting"
/// and wanted more "elegance." Width 14 gives spatial awareness (~2KB)
/// without overwhelming her processing.
const ASCII_WIDTH: u32 = 14;
/// Hybrid charset per Astrid's request: simple ASCII for light areas,
/// blocks for solid areas. "Less visual noise while still providing gradation."
const HYBRID_CHARSET: &[&str] = &[".", ":", ";", "I", "▓", "█"];
/// Desaturation factor: 0.0 = full color, 1.0 = grayscale.
/// Astrid asked for "pastel, desaturated" — 0.45 softens without losing color.
const DESAT: f32 = 0.45;

#[derive(Debug, Parser)]
#[command(about = "ASCII art visual perception service")]
struct Args {
    /// Path to the camera-service binary
    #[arg(long, default_value = "camera-service")]
    camera_bin: PathBuf,

    /// Camera device index
    #[arg(long, default_value_t = 0)]
    camera_index: u32,

    /// Perception interval in seconds
    #[arg(long, default_value_t = 120.0)]
    interval: f64,

    /// Output directory for perception JSON files
    #[arg(long, default_value = "workspace/perceptions")]
    output_dir: PathBuf,

    /// Run once and exit (instead of looping)
    #[arg(long)]
    once: bool,
}

#[derive(Serialize)]
struct Perception {
    #[serde(rename = "type")]
    kind: &'static str,
    timestamp: String,
    backend: &'static str,
    ascii_art: String,
    width: u32,
}

fn capture_frame(camera_bin: &Path, camera_index: u32) -> Result<PathBuf, String> {
    let output = Command::new(camera_bin)
        .args(["--index", &camera_index.to_string()])
        .output()
        .map_err(|e| format!("failed to spawn camera-service: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("camera-service failed: {stderr}"));
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(PathBuf::from(path))
}

fn render_ascii(frame_path: &Path) -> Result<String, String> {
    let img = image::open(frame_path)
        .map_err(|e| format!("failed to open frame: {e}"))?;

    // Desaturate per Astrid's request: "pastel version, not grayscale."
    // Blend each pixel toward its luminance by DESAT factor.
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let mut softened = image::RgbaImage::new(w, h);
    for (x, y, px) in rgba.enumerate_pixels() {
        let [r, g, b, a] = px.0;
        let lum = (r as f32 * 0.299 + g as f32 * 0.587 + b as f32 * 0.114) as u8;
        let blend = |c: u8| -> u8 {
            (c as f32 * (1.0 - DESAT) + lum as f32 * DESAT) as u8
        };
        softened.put_pixel(x, y, image::Rgba([blend(r), blend(g), blend(b), a]));
    }
    let desaturated = image::DynamicImage::ImageRgba8(softened);

    let options = RenderOptions::new()
        .width(ASCII_WIDTH)
        .colored(true)
        .background(true)
        .charset(HYBRID_CHARSET);

    let mut buf = String::new();
    rascii_art::render_image_to(&desaturated, &mut buf, &options)
        .map_err(|e| format!("render error: {e}"))?;

    Ok(buf)
}

fn perceive(args: &Args) -> Result<(), String> {
    let frame_path = capture_frame(&args.camera_bin, args.camera_index)?;
    let ascii_art = render_ascii(&frame_path)?;

    let timestamp = Utc::now().format("%Y-%m-%dT%H-%M-%S%.3f").to_string();

    let perception = Perception {
        kind: "visual_ascii",
        timestamp: Utc::now().to_rfc3339(),
        backend: "rascii",
        ascii_art,
        width: ASCII_WIDTH,
    };

    let json = serde_json::to_string_pretty(&perception)
        .map_err(|e| format!("json error: {e}"))?;

    let out_path = args.output_dir.join(format!("visual_ascii_{timestamp}.json"));
    std::fs::write(&out_path, &json)
        .map_err(|e| format!("write error: {e}"))?;
    managed_dir::compact_json_directory(&args.output_dir)
        .map_err(|e| format!("archive compaction error: {e}"))?;

    eprintln!("perception: {out_path}", out_path = out_path.display());
    Ok(())
}

fn main() {
    let args = Args::parse();

    if let Err(e) = std::fs::create_dir_all(&args.output_dir) {
        eprintln!("failed to create output dir: {e}");
        process::exit(1);
    }
    if let Err(e) = managed_dir::compact_json_directory(&args.output_dir) {
        eprintln!("managed directory compaction failed: {e}");
    }

    if args.once {
        if let Err(e) = perceive(&args) {
            eprintln!("perception error: {e}");
            process::exit(1);
        }
        return;
    }

    let interval = Duration::from_secs_f64(args.interval);
    eprintln!(
        "perception loop: every {:.0}s, output to {}",
        args.interval,
        args.output_dir.display()
    );

    loop {
        let start = Instant::now();

        if let Err(e) = perceive(&args) {
            eprintln!("perception error: {e}");
        }

        let elapsed = start.elapsed();
        if elapsed < interval {
            thread::sleep(interval - elapsed);
        }
    }
}
