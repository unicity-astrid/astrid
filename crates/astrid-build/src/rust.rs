//! Rust capsule builder — compiles a Rust crate to `wasm32-wasip1` and packages it.

use crate::archiver::pack_capsule_archive;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// Build a Rust capsule from a crate directory.
///
/// 1. `cargo build --target wasm32-wasip1 --release`
/// 2. Extract capsule description via Extism (`astrid_export_schemas`)
/// 3. Merge description into `Capsule.toml`
/// 4. Pack into `.capsule` archive
pub(crate) fn build(dir: &Path, output: Option<&str>) -> Result<()> {
    info!("Building Rust WASM capsule from {}", dir.display());

    // 1. Verify cargo is available
    let cargo_check = std::process::Command::new("cargo")
        .arg("--version")
        .output();
    if cargo_check.is_err() {
        bail!("`cargo` is not installed or not in PATH. Rust compilation failed.");
    }

    // 2. Parse Cargo Metadata to get the exact artifact name
    let meta = cargo_metadata::MetadataCommand::new()
        .current_dir(dir)
        .no_deps()
        .exec()
        .context("Failed to parse Cargo metadata")?;

    let package = meta
        .packages
        .iter()
        .find(|p| {
            if let Some(parent) = p.manifest_path.parent()
                && let Ok(canon_parent) = parent.as_std_path().canonicalize()
                && let Ok(canon_dir) = dir.canonicalize()
            {
                return canon_parent == canon_dir;
            }
            false
        })
        .or_else(|| meta.root_package())
        .context("No package found matching the target directory in Cargo.toml")?;

    let crate_name = package.name.clone();
    let package_version = package.version.to_string();
    let wasm_name = crate_name.replace('-', "_");

    // 3. Compile the WASM target
    info!("   Compiling target wasm32-wasip1...");
    let status = std::process::Command::new("cargo")
        .current_dir(dir)
        .args(["build", "--target", "wasm32-wasip1", "--release"])
        .status()
        .context("Failed to spawn cargo build")?;

    if !status.success() {
        bail!(
            "Cargo build failed. Ensure you have the target installed: `rustup target add wasm32-wasip1`"
        );
    }

    // 4. Locate the compiled WASM binary
    let mut wasm_path = dir
        .join("target")
        .join("wasm32-wasip1")
        .join("release")
        .join(format!("{wasm_name}.wasm"));

    if !wasm_path.exists() {
        wasm_path = meta
            .workspace_root
            .into_std_path_buf()
            .join("target")
            .join("wasm32-wasip1")
            .join("release")
            .join(format!("{wasm_name}.wasm"));
    }

    if !wasm_path.exists() {
        bail!(
            "Could not locate compiled WASM binary at {}",
            wasm_path.display()
        );
    }

    // 5. Extract capsule description using Extism
    let capsule_description = extract_capsule_description(&wasm_path);

    // 6. Merge with developer's Capsule.toml
    let base_toml_path = dir.join("Capsule.toml");
    let mut toml_doc = if base_toml_path.exists() {
        let content = fs::read_to_string(&base_toml_path).context("Failed to read Capsule.toml")?;
        content
            .parse::<toml_edit::DocumentMut>()
            .context("Failed to parse Capsule.toml")?
    } else {
        create_default_manifest(&crate_name, &package_version, &wasm_name)
    };

    // Inject capsule description into package.description if not already set
    if let Some(desc) = &capsule_description
        && let Some(pkg) = toml_doc.get_mut("package")
        && let Some(table) = pkg.as_table_mut()
    {
        let existing = table
            .get("description")
            .and_then(toml_edit::Item::as_str)
            .unwrap_or("");
        if existing.is_empty() {
            table.insert("description", toml_edit::value(desc.as_str()));
        }
    }

    let toml_content = toml_doc.to_string();

    // 7. Pack the archive
    let out_dir = match output {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir()?.join("dist"),
    };

    if !out_dir.exists() {
        fs::create_dir_all(&out_dir)?;
    }

    let out_file = out_dir.join(format!("{crate_name}.capsule"));
    pack_capsule_archive(&out_file, &toml_content, Some(&wasm_path), dir, &[])?;

    info!("Successfully built Rust capsule: {}", out_file.display());
    Ok(())
}

/// Extract capsule description from a compiled WASM binary.
///
/// Extract capsule description from the compiled WASM binary.
///
/// Previously called `astrid_export_schemas` via Extism. With the Component
/// Model migration, capsule metadata is extracted from `Capsule.toml` instead.
/// Returns `None` — description is set from the manifest.
fn extract_capsule_description(_wasm_path: &Path) -> Option<String> {
    // Component Model capsules don't export `astrid_export_schemas`.
    // Description comes from Capsule.toml [package] section instead.
    None
}

fn create_default_manifest(
    crate_name: &str,
    package_version: &str,
    wasm_name: &str,
) -> toml_edit::DocumentMut {
    let mut doc = toml_edit::DocumentMut::new();

    let mut package = toml_edit::Table::new();
    package.insert("name", toml_edit::value(crate_name));
    package.insert("version", toml_edit::value(package_version));
    package.insert("description", toml_edit::value(""));
    doc.insert("package", toml_edit::Item::Table(package));

    let mut comp = toml_edit::Table::new();
    comp.insert("id", toml_edit::value(crate_name));
    comp.insert("file", toml_edit::value(format!("{wasm_name}.wasm")));
    comp.insert("type", toml_edit::value("executable"));

    let mut comp_arr = toml_edit::ArrayOfTables::new();
    comp_arr.push(comp);
    doc.insert("component", toml_edit::Item::ArrayOfTables(comp_arr));

    doc
}
