use anyhow::{Context, Result};
use flate2::Compression;
use flate2::write::GzEncoder;
use std::fs::File;
use std::path::Path;
use tracing::info;

/// Packages a set of files and directories into a single `.capsule` (tar.gz) archive.
pub(crate) fn pack_capsule_archive(
    output_path: &Path,
    manifest_content: &str,
    wasm_path: Option<&Path>,
    base_dir: &Path,
    additional_files: &[&Path],
) -> Result<()> {
    info!("📦 Packing capsule archive into {}", output_path.display());

    let tar_gz = File::create(output_path)
        .with_context(|| format!("Failed to create archive file: {}", output_path.display()))?;

    let enc = GzEncoder::new(tar_gz, Compression::default());
    let mut tar = tar::Builder::new(enc);

    // 1. Write the synthesized Capsule.toml directly from memory
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest_content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, "Capsule.toml", manifest_content.as_bytes())
        .context("Failed to write Capsule.toml to archive")?;

    // 2. Append the WASM binary (if present)
    if let Some(wasm) = wasm_path {
        if wasm.exists() {
            let mut wasm_file = File::open(wasm).with_context(|| {
                format!("Failed to open WASM binary for packing: {}", wasm.display())
            })?;
            let file_name = wasm.file_name().unwrap_or_default();
            tar.append_file(file_name, &mut wasm_file)
                .with_context(|| {
                    format!(
                        "Failed to append WASM binary to archive: {}",
                        wasm.display()
                    )
                })?;
        } else {
            anyhow::bail!("WASM binary not found at {}", wasm.display());
        }
    }

    // 3. Append any additional contextual files (like READMEs, skill files, etc.)
    for file_path in additional_files {
        if file_path.exists() {
            let rel_path = file_path
                .strip_prefix(base_dir)
                .unwrap_or(Path::new(file_path.file_name().unwrap_or_default()));

            if file_path.is_dir() {
                tar.append_dir_all(rel_path, file_path).with_context(|| {
                    format!(
                        "Failed to append directory to archive: {}",
                        file_path.display()
                    )
                })?;
            } else {
                let mut f = File::open(file_path).with_context(|| {
                    format!("Failed to open file for packing: {}", file_path.display())
                })?;
                tar.append_file(rel_path, &mut f).with_context(|| {
                    format!("Failed to append file to archive: {}", file_path.display())
                })?;
            }
        }
    }

    tar.finish().context("Failed to finalize capsule archive")?;

    info!("✅ Capsule packaged successfully!");
    Ok(())
}
