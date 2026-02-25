//! Build script for astrid-openclaw.
//!
//! Handles the `QuickJS` WASM kernel:
//! - If `kernel/engine.wasm` exists, copies it to `OUT_DIR` and verifies blake3 hash
//! - If not, generates a minimal placeholder WASM module
//!
//! The compiler detects the placeholder at runtime and errors with build instructions.

use std::path::Path;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");

    let kernel_src = Path::new(&manifest_dir).join("kernel/engine.wasm");
    let kernel_dst = Path::new(&out_dir).join("engine.wasm");

    println!("cargo:rerun-if-changed=kernel/engine.wasm");
    println!("cargo:rerun-if-changed=kernel/engine.wasm.blake3");

    if kernel_src.exists() {
        // Copy real kernel to OUT_DIR
        let kernel_bytes = std::fs::read(&kernel_src).expect("failed to read kernel/engine.wasm");

        // Validate WASM magic
        assert!(
            kernel_bytes.len() >= 8 && kernel_bytes[..4] == *b"\0asm",
            "kernel/engine.wasm is not a valid WASM module"
        );

        // Verify blake3 hash if hash file exists
        let hash_file = Path::new(&manifest_dir).join("kernel/engine.wasm.blake3");
        if hash_file.exists() {
            let hash_content =
                std::fs::read_to_string(&hash_file).expect("failed to read engine.wasm.blake3");
            // Format: "<hash>  <filename>"
            if let Some(expected_hash) = hash_content.split_whitespace().next() {
                let actual_hash = blake3::hash(&kernel_bytes).to_hex().to_string();
                assert!(
                    actual_hash == expected_hash,
                    "kernel blake3 hash mismatch!\n  expected: {expected_hash}\n  actual:   {actual_hash}\n\
                     Re-run: b3sum kernel/engine.wasm > kernel/engine.wasm.blake3"
                );
            }
        }

        std::fs::write(&kernel_dst, &kernel_bytes).expect("failed to write kernel to OUT_DIR");
        println!(
            "cargo:warning=Using QuickJS kernel: {} ({} bytes)",
            kernel_src.display(),
            kernel_bytes.len()
        );
    } else {
        // Write a minimal placeholder WASM module (8 bytes: magic + version)
        // The compiler detects this at runtime via size check and errors helpfully.
        let placeholder = b"\0asm\x01\x00\x00\x00";
        std::fs::write(&kernel_dst, placeholder).expect("failed to write placeholder kernel");
        println!(
            "cargo:warning=QuickJS kernel not found at kernel/engine.wasm — using placeholder. \
             Build the real kernel with: ./scripts/build-quickjs-kernel.sh"
        );
    }
}
