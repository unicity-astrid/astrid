use crate::types::{
    Applicability, CargoMessage, CompilerMessage, DiagLevel, Diagnostic, FmtIssue, StatusSummary,
    Suggestion,
};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

pub struct DiagnosticStore {
    pub diagnostics: Vec<Diagnostic>,
    pub fmt_issues: Vec<FmtIssue>,
    pub checking: AtomicBool,
    pub last_check: Option<Instant>,
    pub generation: u64,

    // Delta tracking — counts from previous check
    pub prev_error_count: usize,
    pub prev_warning_count: usize,
    pub prev_fmt_count: usize,

    pub workspace_root: PathBuf,
}

impl DiagnosticStore {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            diagnostics: Vec::new(),
            fmt_issues: Vec::new(),
            checking: AtomicBool::new(false),
            last_check: None,
            generation: 0,
            prev_error_count: 0,
            prev_warning_count: 0,
            prev_fmt_count: 0,
            workspace_root,
        }
    }

    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.level == DiagLevel::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.level == DiagLevel::Warning)
            .count()
    }

    pub fn status_line(&self) -> String {
        if self.checking.load(Ordering::Relaxed) {
            return "checking...".to_string();
        }

        let errors = self.error_count();
        let warnings = self.warning_count();
        let fmt = self.fmt_issues.len();

        if errors == 0 && warnings == 0 && fmt == 0 {
            if self.last_check.is_some() {
                return "clean".to_string();
            } else {
                return "no checks run yet".to_string();
            }
        }

        let mut parts = Vec::new();
        if errors > 0 {
            parts.push(format!(
                "{errors} error{}",
                if errors == 1 { "" } else { "s" }
            ));
        }
        if warnings > 0 {
            parts.push(format!(
                "{warnings} warning{}",
                if warnings == 1 { "" } else { "s" }
            ));
        }
        if fmt > 0 {
            parts.push(format!(
                "{fmt} fmt issue{}",
                if fmt == 1 { "" } else { "s" }
            ));
        }
        parts.join(", ")
    }

    pub fn delta_line(&self) -> String {
        if self.generation == 0 {
            return "no previous check to compare against".to_string();
        }

        let cur_errors = self.error_count();
        let cur_warnings = self.warning_count();
        let cur_fmt = self.fmt_issues.len();

        let err_delta = cur_errors as i64 - self.prev_error_count as i64;
        let warn_delta = cur_warnings as i64 - self.prev_warning_count as i64;
        let fmt_delta = cur_fmt as i64 - self.prev_fmt_count as i64;

        if err_delta == 0 && warn_delta == 0 && fmt_delta == 0 {
            return "no change since last check".to_string();
        }

        let mut parts = Vec::new();
        if err_delta != 0 {
            let sign = if err_delta > 0 { "+" } else { "" };
            parts.push(format!("{sign}{err_delta} errors"));
        }
        if warn_delta != 0 {
            let sign = if warn_delta > 0 { "+" } else { "" };
            parts.push(format!("{sign}{warn_delta} warnings"));
        }
        if fmt_delta != 0 {
            let sign = if fmt_delta > 0 { "+" } else { "" };
            parts.push(format!("{sign}{fmt_delta} fmt issues"));
        }
        parts.join(", ")
    }

    /// Run clippy + fmt checks, update the store, write status file.
    pub fn run_checks(&mut self) {
        self.checking.store(true, Ordering::Relaxed);

        // Save previous counts for delta
        self.prev_error_count = self.error_count();
        self.prev_warning_count = self.warning_count();
        self.prev_fmt_count = self.fmt_issues.len();

        // Run clippy
        let diags = run_clippy(&self.workspace_root);
        self.diagnostics = diags;

        // Run fmt check
        let fmt = run_fmt_check(&self.workspace_root);
        self.fmt_issues = fmt;

        self.last_check = Some(Instant::now());
        self.generation += 1;
        self.checking.store(false, Ordering::Relaxed);

        // Write status file for hook consumption
        self.write_status_file();

        info!(
            errors = self.error_count(),
            warnings = self.warning_count(),
            fmt_issues = self.fmt_issues.len(),
            generation = self.generation,
            "Check complete"
        );
    }

    pub fn errors(
        &self,
        file_filter: Option<&str>,
        crate_filter: Option<&str>,
    ) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.level == DiagLevel::Error)
            .filter(|d| match file_filter {
                Some(f) => d.file.contains(f),
                None => true,
            })
            .filter(|d| match crate_filter {
                Some(c) => extract_crate_from_path(&d.file).is_some_and(|cn| cn.contains(c)),
                None => true,
            })
            .collect()
    }

    pub fn warnings(
        &self,
        file_filter: Option<&str>,
        crate_filter: Option<&str>,
    ) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.level == DiagLevel::Warning)
            .filter(|d| match file_filter {
                Some(f) => d.file.contains(f),
                None => true,
            })
            .filter(|d| match crate_filter {
                Some(c) => extract_crate_from_path(&d.file).is_some_and(|cn| cn.contains(c)),
                None => true,
            })
            .collect()
    }

    pub fn get_suggestion(&self, diag_id: u64) -> Option<&Diagnostic> {
        self.diagnostics
            .iter()
            .find(|d| d.id == diag_id && !d.suggestions.is_empty())
    }

    /// Get diagnostics only in files changed since HEAD.
    pub fn my_errors(&self) -> Vec<&Diagnostic> {
        let changed_files = git_changed_files(&self.workspace_root);
        if changed_files.is_empty() {
            return Vec::new();
        }

        self.diagnostics
            .iter()
            .filter(|d| {
                d.level == DiagLevel::Error
                    && changed_files.iter().any(|f| d.file.contains(f.as_str()))
            })
            .collect()
    }

    fn write_status_file(&self) {
        let status_dir = self.workspace_root.join("target").join("cargo-diag");
        if std::fs::create_dir_all(&status_dir).is_err() {
            warn!("Failed to create status directory");
            return;
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let summary = StatusSummary {
            summary: self.status_line(),
            error_count: self.error_count(),
            warning_count: self.warning_count(),
            fmt_count: self.fmt_issues.len(),
            checking: self.checking.load(Ordering::Relaxed),
            timestamp_epoch_ms: timestamp,
        };

        let json = match serde_json::to_string_pretty(&summary) {
            Ok(j) => j,
            Err(e) => {
                warn!(error = %e, "Failed to serialize status");
                return;
            },
        };

        let path = status_dir.join("status.json");
        if let Err(e) = std::fs::write(&path, json) {
            warn!(error = %e, "Failed to write status file");
        }
    }
}

fn run_clippy(workspace_root: &Path) -> Vec<Diagnostic> {
    info!("Running cargo clippy...");
    let start = Instant::now();

    let target_dir = workspace_root.join("target").join("cargo-diag");
    if let Ok(size) = get_dir_size(&target_dir) {
        if size > 2_000_000_000 { // 2 GB
            info!("cargo-diag target directory exceeded 2GB ({} bytes), cleaning...", size);
            let _ = std::fs::remove_dir_all(&target_dir);
        }
    }

    let output = Command::new("cargo")
        .env("CARGO_TARGET_DIR", "target/cargo-diag")
        .args([
            "clippy",
            "--workspace",
            "--message-format=json",
            "--",
            "-D",
            "warnings",
        ])
        .current_dir(workspace_root)
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            warn!(error = %e, "Failed to run cargo clippy, falling back to cargo check");
            return run_check_fallback(workspace_root);
        },
    };

    let elapsed = start.elapsed();
    debug!(elapsed_ms = elapsed.as_millis(), "Clippy finished");

    parse_cargo_diagnostics(&output.stdout)
}

fn run_check_fallback(workspace_root: &Path) -> Vec<Diagnostic> {
    info!("Running cargo check (fallback)...");

    let output = Command::new("cargo")
        .env("CARGO_TARGET_DIR", "target/cargo-diag")
        .args(["check", "--workspace", "--message-format=json"])
        .current_dir(workspace_root)
        .output();

    match output {
        Ok(o) => parse_cargo_diagnostics(&o.stdout),
        Err(e) => {
            warn!(error = %e, "Failed to run cargo check");
            Vec::new()
        },
    }
}

fn parse_cargo_diagnostics(stdout: &[u8]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut next_id: u64 = 1;

    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        if line.is_empty() {
            continue;
        }

        let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) else {
            continue;
        };

        if msg.reason != "compiler-message" {
            continue;
        }

        let Some(compiler_msg) = msg.message else {
            continue;
        };

        let level = match compiler_msg.level.as_str() {
            "error" => DiagLevel::Error,
            "warning" => DiagLevel::Warning,
            "note" => DiagLevel::Note,
            "help" => DiagLevel::Help,
            "failure-note" => DiagLevel::FailureNote,
            _ => continue,
        };

        // Only keep errors and warnings
        if level != DiagLevel::Error && level != DiagLevel::Warning {
            continue;
        }

        // Find the primary span
        let primary_span = compiler_msg.spans.iter().find(|s| s.is_primary);
        let (file, line_num, column) = match primary_span {
            Some(s) => (s.file_name.clone(), s.line_start, s.column_start),
            None => continue,
        };

        let code = compiler_msg.code.as_ref().map(|c| c.code.clone());
        let rendered = compiler_msg.rendered.clone().unwrap_or_default();

        // Extract suggestions from children
        let suggestions = extract_suggestions(&compiler_msg);

        diagnostics.push(Diagnostic {
            id: next_id,
            level,
            message: compiler_msg.message,
            code,
            file,
            line: line_num,
            column,
            rendered,
            suggestions,
        });
        next_id += 1;
    }

    diagnostics
}

fn extract_suggestions(msg: &CompilerMessage) -> Vec<Suggestion> {
    let mut suggestions = Vec::new();

    for child in &msg.children {
        for span in &child.spans {
            if let Some(ref replacement) = span.suggested_replacement {
                let applicability = match span
                    .suggestion_applicability
                    .as_deref()
                    .unwrap_or("Unspecified")
                {
                    "MachineApplicable" => Applicability::MachineApplicable,
                    "MaybeIncorrect" => Applicability::MaybeIncorrect,
                    "HasPlaceholders" => Applicability::HasPlaceholders,
                    _ => Applicability::Unspecified,
                };

                suggestions.push(Suggestion {
                    message: child.message.clone(),
                    replacement: replacement.clone(),
                    file: span.file_name.clone(),
                    line_start: span.line_start,
                    line_end: span.line_end,
                    col_start: span.column_start,
                    col_end: span.column_end,
                    applicability,
                });
            }
        }

        // Recurse into nested children
        suggestions.extend(extract_suggestions(child));
    }

    suggestions
}

fn run_fmt_check(workspace_root: &Path) -> Vec<FmtIssue> {
    debug!("Running cargo fmt --check...");

    let output = Command::new("cargo")
        .env("CARGO_TARGET_DIR", "target/cargo-diag")
        .args(["fmt", "--all", "--", "--check"])
        .current_dir(workspace_root)
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            warn!(error = %e, "Failed to run cargo fmt");
            return Vec::new();
        },
    };

    if output.status.success() {
        return Vec::new();
    }

    // Parse stdout for "Diff in" lines to extract file paths
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("Diff in ") {
            // Format: "Diff in /path/to/file.rs at line N:"
            if let Some(path) = rest.split(" at ").next() {
                let relative = path
                    .strip_prefix(workspace_root.to_str().unwrap_or(""))
                    .unwrap_or(path)
                    .trim_start_matches('/');
                if seen.insert(relative.to_string()) {
                    files.push(FmtIssue {
                        file: relative.to_string(),
                    });
                }
            }
        }
    }

    files
}

fn git_changed_files(workspace_root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["diff", "--name-only", "HEAD"])
        .current_dir(workspace_root)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => {
            // Also try unstaged changes
            return Vec::new();
        },
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files: Vec<String> = stdout
        .lines()
        .filter(|l| l.ends_with(".rs"))
        .map(|l| l.to_string())
        .collect();

    // Also include staged but uncommitted changes
    if let Ok(staged) = Command::new("git")
        .args(["diff", "--name-only", "--cached"])
        .current_dir(workspace_root)
        .output()
    {
        if staged.status.success() {
            let staged_stdout = String::from_utf8_lossy(&staged.stdout);
            for line in staged_stdout.lines() {
                if line.ends_with(".rs") && !files.contains(&line.to_string()) {
                    files.push(line.to_string());
                }
            }
        }
    }

    files
}

/// Extract crate name from a file path like "crates/astralis-core/src/lib.rs" → "astralis-core"
fn extract_crate_from_path(path: &str) -> Option<&str> {
    let path = path.strip_prefix("crates/")?;
    path.split('/').next()
}

fn get_dir_size(path: &Path) -> std::io::Result<u64> {
    let mut size = 0;
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                size += get_dir_size(&path)?;
            } else {
                size += entry.metadata()?.len();
            }
        }
    }
    Ok(size)
}
