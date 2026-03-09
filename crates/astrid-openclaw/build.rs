//! Build script for astrid-openclaw.
//!
//! Handles the `QuickJS` WASM kernel:
//! - If `kernel/engine.wasm` exists, copies it to `OUT_DIR` and verifies blake3 hash
//! - If not, attempts to auto-build the kernel from source (extism/js-pdk)
//! - Falls back to a placeholder WASM module if auto-build fails
//!
//! The compiler detects the placeholder at runtime via size check and errors with
//! build instructions.

use std::path::{Path, PathBuf};
use std::process::Command;

const JS_PDK_REPO: &str = "https://github.com/nicholasgasior/extism-js.git";
// NOTE: This is a maintained fork of extism/js-pdk with additional patches.
// The official repo is https://github.com/extism/js-pdk.git — switch if
// upstream merges the required changes.
const JS_PDK_TAG: &str = "v1.6.0";

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");

    let kernel_src = Path::new(&manifest_dir).join("kernel/engine.wasm");
    let kernel_dst = Path::new(&out_dir).join("engine.wasm");

    println!("cargo:rerun-if-changed=kernel/engine.wasm");
    println!("cargo:rerun-if-changed=kernel/engine.wasm.blake3");

    if kernel_src.exists() {
        install_existing_kernel(&kernel_src, &kernel_dst, &manifest_dir);
        return;
    }

    // Kernel doesn't exist — try to auto-build (writes directly to OUT_DIR)
    println!("cargo:warning=QuickJS kernel not found — attempting auto-build...");

    if auto_build_kernel(&kernel_dst, &out_dir) {
        return;
    }

    // Auto-build failed — write placeholder
    write_placeholder(&kernel_dst);
}

/// Copy an existing kernel to `OUT_DIR` with WASM validation and blake3 verification.
fn install_existing_kernel(kernel_src: &Path, kernel_dst: &Path, manifest_dir: &str) {
    let kernel_bytes = std::fs::read(kernel_src).expect("failed to read kernel/engine.wasm");

    // Validate WASM magic
    assert!(
        kernel_bytes.len() >= 8 && kernel_bytes[..4] == *b"\0asm",
        "kernel/engine.wasm is not a valid WASM module"
    );

    // Verify blake3 hash if hash file exists
    let hash_file = Path::new(manifest_dir).join("kernel/engine.wasm.blake3");
    if hash_file.exists() {
        let hash_content =
            std::fs::read_to_string(&hash_file).expect("failed to read engine.wasm.blake3");
        if let Some(expected_hash) = hash_content.split_whitespace().next() {
            let actual_hash = blake3::hash(&kernel_bytes).to_hex().to_string();
            assert!(
                actual_hash == expected_hash,
                "kernel blake3 hash mismatch!\n  expected: {expected_hash}\n  actual:   {actual_hash}\n\
                 Re-run: b3sum kernel/engine.wasm > kernel/engine.wasm.blake3"
            );
        }
    }

    std::fs::write(kernel_dst, &kernel_bytes).expect("failed to write kernel to OUT_DIR");
    println!(
        "cargo:warning=Using QuickJS kernel: {} ({} bytes)",
        kernel_src.display(),
        kernel_bytes.len()
    );
}

/// Write a minimal placeholder WASM module (8 bytes).
fn write_placeholder(kernel_dst: &Path) {
    let placeholder = b"\0asm\x01\x00\x00\x00";
    std::fs::write(kernel_dst, placeholder).expect("failed to write placeholder kernel");
    println!(
        "cargo:warning=QuickJS kernel auto-build failed — using placeholder. \
         Tier 1 (WASM) compilation will not work. \
         Manual build: ./scripts/build-quickjs-kernel.sh"
    );
}

/// Check if a command exists on PATH.
fn has_command(name: &str) -> bool {
    let lookup = if cfg!(windows) { "where" } else { "which" };
    Command::new(lookup)
        .arg(name)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Run a command, returning true on success. Prints stderr on failure.
fn run_step(description: &str, cmd: &mut Command) -> bool {
    println!("cargo:warning=  [auto-build] {description}...");
    match cmd.output() {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("cargo:warning=  [auto-build] FAILED: {description}");
            for line in stderr.lines().take(5) {
                println!("cargo:warning=    {line}");
            }
            false
        },
        Err(e) => {
            println!("cargo:warning=  [auto-build] FAILED to run: {e}");
            false
        },
    }
}

/// Verify that git, npm, and wasm32-wasip1 target are available.
fn check_prerequisites() -> bool {
    for cmd in ["git", "npm"] {
        if !has_command(cmd) {
            println!("cargo:warning=  [auto-build] '{cmd}' not found — skipping auto-build");
            return false;
        }
    }

    // Ensure wasm32-wasip1 target is available
    let target_check = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output();
    let has_target = target_check
        .as_ref()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("wasm32-wasip1"));

    if !has_target
        && !run_step(
            "installing wasm32-wasip1 target",
            Command::new("rustup").args(["target", "add", "wasm32-wasip1"]),
        )
    {
        return false;
    }

    true
}

/// Attempt to auto-build the `QuickJS` kernel from source.
///
/// Writes the built kernel directly to `kernel_dst` (in `OUT_DIR`).
/// Does NOT modify the source tree.
fn auto_build_kernel(kernel_dst: &Path, out_dir: &str) -> bool {
    if !check_prerequisites() {
        return false;
    }

    // Clone extism-js into a temp build directory
    let build_dir = PathBuf::from(out_dir).join("quickjs-kernel-build");
    if build_dir.exists() {
        let _ = std::fs::remove_dir_all(&build_dir);
    }

    if !run_step(
        &format!("cloning extism-js {JS_PDK_TAG}"),
        Command::new("git").args([
            "clone",
            "--depth",
            "1",
            "--branch",
            JS_PDK_TAG,
            JS_PDK_REPO,
            &build_dir.to_string_lossy(),
        ]),
    ) {
        return false;
    }

    // Install wasi-sdk (required by rquickjs C bindings)
    if !run_step(
        "installing wasi-sdk",
        Command::new("sh")
            .arg("install-wasi-sdk.sh")
            .current_dir(&build_dir),
    ) {
        return false;
    }

    // Build JS prelude
    let prelude_dir = build_dir.join("crates/core/src/prelude");
    if !run_step(
        "npm install (JS prelude)",
        Command::new("npm").arg("install").current_dir(&prelude_dir),
    ) {
        return false;
    }
    if !run_step(
        "npm run build (JS prelude)",
        Command::new("npm")
            .args(["run", "build"])
            .current_dir(&prelude_dir),
    ) {
        return false;
    }

    // Build QuickJS core to wasm32-wasip1
    if !run_step(
        "building QuickJS core (wasm32-wasip1)",
        Command::new("cargo")
            .args(["build", "--release", "--target=wasm32-wasip1"])
            .current_dir(&build_dir),
    ) {
        return false;
    }

    // Locate built WASM
    let built_wasm = build_dir.join("target/wasm32-wasip1/release/js_pdk_core.wasm");
    if !built_wasm.exists() {
        println!(
            "cargo:warning=  [auto-build] Build output not found at {}",
            built_wasm.display()
        );
        return false;
    }

    // Optional: optimize with wasm-opt (write to temp file to avoid corruption)
    if has_command("wasm-opt") {
        let optimized = built_wasm.with_extension("opt.wasm");
        if run_step(
            "optimizing with wasm-opt",
            Command::new("wasm-opt").args([
                "--enable-reference-types",
                "--enable-bulk-memory",
                "--strip",
                "-O3",
                &built_wasm.to_string_lossy(),
                "-o",
                &optimized.to_string_lossy(),
            ]),
        ) && optimized.exists()
        {
            let _ = std::fs::rename(&optimized, &built_wasm);
        } else {
            // Clean up failed optimization output
            let _ = std::fs::remove_file(&optimized);
        }
        // Don't fail if wasm-opt fails — the unoptimized binary works fine
    }

    // Install the built kernel directly to OUT_DIR
    let success = install_built_kernel(&built_wasm, kernel_dst);

    // Clean up build dir regardless of outcome
    let _ = std::fs::remove_dir_all(&build_dir);

    success
}

/// Validate and install a freshly built kernel WASM to `kernel_dst` (in `OUT_DIR`).
///
/// Does NOT write to the source tree — only `OUT_DIR` is modified.
fn install_built_kernel(built_wasm: &Path, kernel_dst: &Path) -> bool {
    let Ok(wasm_bytes) = std::fs::read(built_wasm) else {
        println!("cargo:warning=  [auto-build] Failed to read built WASM");
        return false;
    };

    // Validate it's real WASM (not empty or corrupt)
    if wasm_bytes.len() < 1024 || wasm_bytes[..4] != *b"\0asm" {
        println!("cargo:warning=  [auto-build] Built WASM is invalid or too small");
        return false;
    }

    if std::fs::write(kernel_dst, &wasm_bytes).is_err() {
        println!("cargo:warning=  [auto-build] Failed to write kernel to OUT_DIR");
        return false;
    }

    println!(
        "cargo:warning=  [auto-build] SUCCESS: QuickJS kernel built ({} bytes)",
        wasm_bytes.len()
    );

    true
}
