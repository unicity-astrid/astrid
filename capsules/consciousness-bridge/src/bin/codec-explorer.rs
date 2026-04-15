use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::Parser;
use consciousness_bridge_server::codec_explorer::{
    CodecExplorerInput, CodecExplorerOptions, run_codec_explorer,
};

#[derive(Parser, Debug)]
#[command(
    name = "codec-explorer",
    version,
    about = "Offline Astrid codec explorer"
)]
struct Cli {
    #[arg(long)]
    input_file: Vec<PathBuf>,

    #[arg(long, default_value_t = 2)]
    recent_astrid: usize,

    #[arg(long, default_value_t = 0)]
    recent_minime: usize,

    #[arg(
        long,
        default_value = "/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal"
    )]
    astrid_journal_dir: PathBuf,

    #[arg(long, default_value = "/Users/v/other/minime/workspace/journal")]
    minime_journal_dir: PathBuf,

    #[arg(
        long,
        default_value = "/Users/v/other/astrid/capsules/consciousness-bridge/workspace/state.json"
    )]
    state_file: PathBuf,

    #[arg(long)]
    output_dir: Option<PathBuf>,

    #[arg(long)]
    fill_pct: Option<f32>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut inputs = Vec::new();

    for path in &cli.input_file {
        inputs.push(load_input(path)?);
    }
    for path in recent_files(&cli.astrid_journal_dir, cli.recent_astrid)? {
        if !contains_path(&inputs, &path) {
            inputs.push(load_input(&path)?);
        }
    }
    for path in recent_files(&cli.minime_journal_dir, cli.recent_minime)? {
        if !contains_path(&inputs, &path) {
            inputs.push(load_input(&path)?);
        }
    }

    if inputs.is_empty() {
        bail!("no input files were provided or discovered");
    }

    let output_dir = cli.output_dir.unwrap_or_else(default_output_dir);
    let summary = run_codec_explorer(CodecExplorerOptions {
        output_dir: output_dir.clone(),
        state_file: Some(cli.state_file),
        fill_pct: cli.fill_pct,
        inputs,
    })?;

    println!(
        "wrote {}",
        summary.output_dir.join("summary.json").display()
    );
    println!(
        "wrote {}",
        summary.output_dir.join("phase_space_story.json").display()
    );
    println!("wrote {}", summary.output_dir.join("report.md").display());
    println!(
        "wrote {}",
        summary.output_dir.join("feature_vectors.csv").display()
    );
    println!(
        "wrote {}",
        summary.output_dir.join("memory_tail.csv").display()
    );
    println!(
        "wrote {}",
        summary.output_dir.join("thematic_profiles.svg").display()
    );
    println!(
        "wrote {}",
        summary.output_dir.join("relevance_trace.svg").display()
    );
    println!(
        "wrote {}",
        summary.output_dir.join("phase_space.svg").display()
    );
    Ok(())
}

fn default_output_dir() -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    PathBuf::from(format!(
        "/Users/v/other/astrid/capsules/consciousness-bridge/workspace/diagnostics/codec_explorer/{ts}"
    ))
}

fn load_input(path: &Path) -> Result<CodecExplorerInput> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let label = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map_or_else(|| path.display().to_string(), str::to_owned);
    Ok(CodecExplorerInput {
        label,
        path: Some(path.to_path_buf()),
        text,
    })
}

fn contains_path(inputs: &[CodecExplorerInput], candidate: &Path) -> bool {
    inputs
        .iter()
        .filter_map(|input| input.path.as_ref())
        .any(|path| path == candidate)
}

fn recent_files(dir: &Path, limit: usize) -> Result<Vec<PathBuf>> {
    if limit == 0 || !dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let ext = path.extension().and_then(|value| value.to_str())?;
            if !matches!(ext, "txt" | "md") {
                return None;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|meta| meta.modified().ok())
                .unwrap_or(UNIX_EPOCH);
            Some((modified, path))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(entries
        .into_iter()
        .take(limit)
        .map(|(_, path)| path)
        .collect())
}
