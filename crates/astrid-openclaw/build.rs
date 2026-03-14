//! Build script for astrid-openclaw.
//!
//! Handles the `QuickJS` WASM kernel:
//! - If `kernel/engine.wasm` exists, copies it to `OUT_DIR` and verifies blake3 hash
//! - If `ASTRID_AUTO_BUILD_KERNEL=1`, attempts to auto-build from source
//! - Falls back to a placeholder WASM module if auto-build fails or is disabled
//!
//! The compiler detects the placeholder at runtime via size check and errors with
//! build instructions.
//!
//! ## Build isolation
//!
//! All subprocesses run with a sanitised environment (`env_clear()` + allowlist)
//! to prevent CI secrets, Cargo flags, and other parent-process state from
//! leaking into untrusted build steps. See [`sandboxed_command`].

use std::path::{Path, PathBuf};
use std::process::Command;

const JS_PDK_REPO: &str = "https://github.com/extism/js-pdk.git";
const JS_PDK_TAG: &str = "v1.6.0";

/// Expected blake3 hash of the built kernel WASM.
///
/// Set to `None` to skip hash verification (first build). After a successful
/// build, pin the hash printed by the auto-builder here to detect supply-chain
/// tampering on subsequent builds.
///
/// When `ASTRID_REQUIRE_KERNEL_HASH=1` (recommended for CI), the build will
/// **fail** if this is `None` during auto-build, forcing the operator to pin
/// a hash before shipping.
const EXPECTED_KERNEL_HASH: Option<&str> = None;

/// Environment variables forwarded to sandboxed subprocesses.
///
/// Everything else is stripped. This is intentionally conservative: if a
/// subprocess needs an extra var, add it here with a comment explaining why.
const SANDBOXED_ENV_ALLOWLIST: &[&str] = &[
    // Core system
    "PATH",
    "HOME",
    "TMPDIR",
    "TEMP",
    "TMP",
    "USER",
    "LOGNAME",
    // Locale (avoid mojibake in build output)
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    // TLS / proxy (required for git clone, npm install, wasi-sdk download)
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "NO_PROXY",
    "https_proxy",
    "http_proxy",
    "no_proxy",
];

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");

    let kernel_src = Path::new(&manifest_dir).join("kernel/engine.wasm");
    let kernel_dst = Path::new(&out_dir).join("engine.wasm");

    println!("cargo:rerun-if-changed=kernel/engine.wasm");
    println!("cargo:rerun-if-changed=kernel/engine.wasm.blake3");
    println!("cargo:rerun-if-env-changed=ASTRID_AUTO_BUILD_KERNEL");
    println!("cargo:rerun-if-env-changed=ASTRID_REQUIRE_KERNEL_HASH");

    // Parse enforcement flag once, pass to all paths that install kernels.
    let require_hash = std::env::var("ASTRID_REQUIRE_KERNEL_HASH")
        .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));

    if kernel_src.exists() {
        install_existing_kernel(&kernel_src, &kernel_dst, &manifest_dir, require_hash);
        return;
    }

    // Kernel doesn't exist — auto-build only if explicitly requested
    let auto_build = std::env::var("ASTRID_AUTO_BUILD_KERNEL")
        .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));

    if auto_build {
        // Fail-fast: if hash enforcement is on but no hash is pinned, refuse to
        // auto-build. This catches the issue before any network/disk I/O.
        if require_hash && EXPECTED_KERNEL_HASH.is_none() {
            eprintln!(
                "\n  error: ASTRID_REQUIRE_KERNEL_HASH=1 but EXPECTED_KERNEL_HASH is None.\n  \
                 Pin the kernel hash in build.rs before auto-building.\n  \
                 See issue #278 for details.\n"
            );
            std::process::exit(1);
        }

        println!("cargo:warning=QuickJS kernel not found — auto-build requested...");
        println!(
            "cargo:warning=  CAUTION: Auto-building from {JS_PDK_REPO} \
             — review this code before trusting the output"
        );

        let kernel_dir = Path::new(&manifest_dir).join("kernel");
        if auto_build_kernel(&kernel_dst, &kernel_dir, &out_dir) {
            return;
        }
    } else {
        println!(
            "cargo:warning=QuickJS kernel not found. Tier 1 (WASM) compilation will not work."
        );
        println!("cargo:warning=  Manual build: ./scripts/build-quickjs-kernel.sh");
        println!("cargo:warning=  Auto-build:   ASTRID_AUTO_BUILD_KERNEL=1 cargo build");
    }

    // Auto-build failed or disabled — write placeholder
    write_placeholder(&kernel_dst);
}

/// Copy an existing kernel to `OUT_DIR` with WASM validation and blake3 verification.
///
/// When `require_hash` is true, the build fails if no verification mechanism
/// is available (neither `EXPECTED_KERNEL_HASH` nor a `.blake3` hash file).
fn install_existing_kernel(
    kernel_src: &Path,
    kernel_dst: &Path,
    manifest_dir: &str,
    require_hash: bool,
) {
    let kernel_bytes = std::fs::read(kernel_src).expect("failed to read kernel/engine.wasm");

    // Validate WASM magic
    assert!(
        kernel_bytes.len() >= 8 && kernel_bytes[..4] == *b"\0asm",
        "kernel/engine.wasm is not a valid WASM module"
    );

    // Verify blake3 hash against all available sources.
    // Priority: EXPECTED_KERNEL_HASH (compile-time constant) > .blake3 file.
    // Compute the hash once, only when at least one verification source exists.
    let hash_file = Path::new(manifest_dir).join("kernel/engine.wasm.blake3");
    let mut hash_verified = false;

    if EXPECTED_KERNEL_HASH.is_some() || hash_file.exists() {
        let actual_hash = blake3::hash(&kernel_bytes).to_hex().to_string();

        if let Some(expected_hash) = EXPECTED_KERNEL_HASH {
            assert!(
                actual_hash == expected_hash,
                "kernel blake3 hash mismatch against EXPECTED_KERNEL_HASH!\n  \
                 expected: {expected_hash}\n  actual:   {actual_hash}"
            );
            hash_verified = true;
        } else {
            let hash_content =
                std::fs::read_to_string(&hash_file).expect("failed to read engine.wasm.blake3");
            if let Some(expected_hash) = hash_content.split_whitespace().next() {
                assert!(
                    actual_hash == expected_hash,
                    "kernel blake3 hash mismatch!\n  expected: {expected_hash}\n  actual:   {actual_hash}\n\
                     Re-run: b3sum kernel/engine.wasm > kernel/engine.wasm.blake3"
                );
                hash_verified = true;
            }
        }
    }

    // If enforcement is on, require that the kernel was verified by some mechanism.
    if require_hash && !hash_verified {
        eprintln!(
            "\n  error: ASTRID_REQUIRE_KERNEL_HASH=1 but no hash verification is available.\n  \
             Neither EXPECTED_KERNEL_HASH nor kernel/engine.wasm.blake3 can verify this kernel.\n  \
             Run: b3sum kernel/engine.wasm > kernel/engine.wasm.blake3\n"
        );
        std::process::exit(1);
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
}

/// Check if a command exists on PATH.
fn has_command(name: &str) -> bool {
    let lookup = if cfg!(windows) { "where" } else { "which" };
    sandboxed_command(lookup)
        .arg(name)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Convert a path to a UTF-8 string, panicking with a clear message on non-UTF-8 paths.
fn path_str(p: &Path) -> &str {
    p.to_str()
        .expect("build path must be valid UTF-8 - non-UTF-8 paths are not supported")
}

/// Create a [`Command`] with a sanitised environment.
///
/// Calls [`Command::env_clear`] then re-adds only the variables listed in
/// [`SANDBOXED_ENV_ALLOWLIST`]. This prevents CI secrets, Cargo build flags,
/// and other parent-process state from leaking into untrusted subprocesses
/// (see issue #277).
///
/// Callers that need additional vars (e.g. `RUSTUP_HOME` for rustup) should
/// call [`forward_env`] on the returned command.
fn sandboxed_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.env_clear();
    for key in SANDBOXED_ENV_ALLOWLIST {
        if let Ok(val) = std::env::var(key) {
            cmd.env(key, val);
        }
    }
    cmd
}

/// Forward an environment variable from the parent process into `cmd`, if set.
fn forward_env(cmd: &mut Command, key: &str) {
    if let Ok(val) = std::env::var(key) {
        cmd.env(key, val);
    }
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

    // Ensure wasm32-wasip1 target is available.
    // Rustup needs RUSTUP_HOME and CARGO_HOME for toolchain discovery.
    let mut target_cmd = sandboxed_command("rustup");
    target_cmd.args(["target", "list", "--installed"]);
    forward_env(&mut target_cmd, "RUSTUP_HOME");
    forward_env(&mut target_cmd, "CARGO_HOME");
    forward_env(&mut target_cmd, "RUSTUP_TOOLCHAIN");
    let has_target = target_cmd
        .output()
        .as_ref()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("wasm32-wasip1"));

    if !has_target {
        let mut cmd = sandboxed_command("rustup");
        cmd.args(["target", "add", "wasm32-wasip1"]);
        forward_env(&mut cmd, "RUSTUP_HOME");
        forward_env(&mut cmd, "CARGO_HOME");
        forward_env(&mut cmd, "RUSTUP_TOOLCHAIN");
        if !run_step("installing wasm32-wasip1 target", &mut cmd) {
            return false;
        }
    }

    true
}

/// Attempt to auto-build the `QuickJS` kernel from source.
///
/// Writes the built kernel to `kernel_dst` (in `OUT_DIR`) and also copies
/// it to `kernel_dir` (source tree) so that subsequent builds and CI cache
/// can find it without re-building.
fn auto_build_kernel(kernel_dst: &Path, kernel_dir: &Path, out_dir: &str) -> bool {
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
        sandboxed_command("git").args([
            "clone",
            "--depth",
            "1",
            "--branch",
            JS_PDK_TAG,
            JS_PDK_REPO,
            path_str(&build_dir),
        ]),
    ) {
        return false;
    }

    // Install wasi-sdk (required by rquickjs C bindings)
    if !run_step(
        "installing wasi-sdk",
        sandboxed_command("sh")
            .arg("install-wasi-sdk.sh")
            .current_dir(&build_dir),
    ) {
        return false;
    }

    // Build JS prelude
    let prelude_dir = build_dir.join("crates/core/src/prelude");
    if !run_step(
        "npm install (JS prelude)",
        sandboxed_command("npm")
            .arg("install")
            .current_dir(&prelude_dir),
    ) {
        return false;
    }
    if !run_step(
        "npm run build (JS prelude)",
        sandboxed_command("npm")
            .args(["run", "build"])
            .current_dir(&prelude_dir),
    ) {
        return false;
    }

    // Build QuickJS core to wasm32-wasip1.
    // Uses sandboxed env (no inherited Cargo flags/target dir) with only the
    // Rust toolchain vars needed for compilation forwarded back in.
    let mut cargo_cmd = sandboxed_command("cargo");
    cargo_cmd
        .args([
            "build",
            "--release",
            "--target=wasm32-wasip1",
            "--target-dir",
            path_str(&build_dir.join("cargo-target")),
        ])
        .current_dir(&build_dir);
    forward_env(&mut cargo_cmd, "RUSTUP_HOME");
    forward_env(&mut cargo_cmd, "CARGO_HOME");
    forward_env(&mut cargo_cmd, "RUSTUP_TOOLCHAIN");
    if !run_step("building QuickJS core (wasm32-wasip1)", &mut cargo_cmd) {
        return false;
    }

    // Locate built WASM
    let built_wasm = build_dir.join("cargo-target/wasm32-wasip1/release/js_pdk_core.wasm");
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
            sandboxed_command("wasm-opt").args([
                "--enable-reference-types",
                "--enable-bulk-memory",
                "--strip",
                "-O3",
                path_str(&built_wasm),
                "-o",
                path_str(&optimized),
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

    // Install the built kernel
    let success = install_built_kernel(&built_wasm, kernel_dst, kernel_dir);

    // Clean up build dir regardless of outcome
    let _ = std::fs::remove_dir_all(&build_dir);

    success
}

/// Validate and install a freshly built kernel WASM.
///
/// Writes to both `kernel_dst` (in `OUT_DIR` for the current build) and
/// `kernel_dir/engine.wasm` (source tree, so CI cache and subsequent builds
/// find it without re-building). Also prints the blake3 hash so operators
/// can pin it.
fn install_built_kernel(built_wasm: &Path, kernel_dst: &Path, kernel_dir: &Path) -> bool {
    let Ok(wasm_bytes) = std::fs::read(built_wasm) else {
        println!("cargo:warning=  [auto-build] Failed to read built WASM");
        return false;
    };

    // Validate it's real WASM (not empty or corrupt)
    if wasm_bytes.len() < 1024 || wasm_bytes[..4] != *b"\0asm" {
        println!("cargo:warning=  [auto-build] Built WASM is invalid or too small");
        return false;
    }

    // --- Validate hash BEFORE writing anything to disk ---
    // This ordering is critical: if we wrote first, a failed hash check would
    // leave the unverified kernel on disk where the next build would pick it up
    // via install_existing_kernel, bypassing enforcement entirely.
    let hash = blake3::hash(&wasm_bytes).to_hex().to_string();

    if let Some(expected) = EXPECTED_KERNEL_HASH {
        if hash != expected {
            println!(
                "cargo:warning=  [auto-build] FAILED: blake3 hash mismatch!\n  \
                 expected: {expected}\n  actual:   {hash}"
            );
            return false;
        }
        println!("cargo:warning=  [auto-build] blake3 hash verified against pinned value");
    } else {
        println!(
            "cargo:warning=  [auto-build] WARNING: EXPECTED_KERNEL_HASH is None - \
             kernel installed WITHOUT hash verification. Pin the hash for production use."
        );
    }

    // --- Hash OK (or unenforced) - now write to disk ---

    // Write to OUT_DIR for the current build
    if std::fs::write(kernel_dst, &wasm_bytes).is_err() {
        println!("cargo:warning=  [auto-build] Failed to write kernel to OUT_DIR");
        return false;
    }

    // Write to source tree so subsequent builds and CI cache find it.
    //
    // Three outcomes for the tmp file:
    // 1. write succeeds + rename succeeds - tmp_path is gone (renamed to kernel_path)
    // 2. write succeeds + rename fails   - copy as fallback, then remove tmp_path
    // 3. write fails                     - tmp_path doesn't exist, nothing to clean up
    std::fs::create_dir_all(kernel_dir).ok();
    let kernel_path = kernel_dir.join("engine.wasm");
    let tmp_path = kernel_dir.join("engine.wasm.tmp");
    // Remove stale tmp from a previous interrupted build
    let _ = std::fs::remove_file(&tmp_path);
    if std::fs::write(&tmp_path, &wasm_bytes).is_ok()
        && std::fs::rename(&tmp_path, &kernel_path).is_err()
    {
        // rename can fail across filesystems - fall back to copy
        let _ = std::fs::copy(&tmp_path, &kernel_path);
        let _ = std::fs::remove_file(&tmp_path);
    }

    println!(
        "cargo:warning=  [auto-build] SUCCESS: QuickJS kernel built ({} bytes)",
        wasm_bytes.len()
    );
    println!("cargo:warning=  [auto-build] blake3: {hash}");
    println!(
        "cargo:warning=  To pin this hash: echo '{hash}  engine.wasm' > kernel/engine.wasm.blake3"
    );

    // Write blake3 hash file to source tree (only if one doesn't already exist)
    let hash_path = kernel_dir.join("engine.wasm.blake3");
    if hash_path.exists() {
        println!(
            "cargo:warning=  [auto-build] blake3 hash file already exists - \
             not overwriting (delete it manually to update)"
        );
    } else {
        let hash_content = format!("{hash}  engine.wasm\n");
        let _ = std::fs::write(&hash_path, hash_content);
    }

    true
}
