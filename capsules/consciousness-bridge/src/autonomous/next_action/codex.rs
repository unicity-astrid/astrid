use std::fs;
use std::path::Path;
use tracing::{info, warn};

use super::mike::is_safe_path;
use super::{ConversationState, NextActionContext, strip_action};
use crate::paths::bridge_paths;

const CODEX_RELAY_URL: &str = "http://127.0.0.1:3040/prompt";
const CODEX_TIMEOUT_SECS: u64 = 60;

pub(super) fn handle_action(
    conv: &mut ConversationState,
    base_action: &str,
    original: &str,
    _ctx: &mut NextActionContext<'_>,
) -> bool {
    match base_action {
        "MIKE_FORK" => {
            let arg = strip_action(original, "MIKE_FORK");
            let parts: Vec<&str> = arg.splitn(2, char::is_whitespace).collect();
            let project = parts.first().copied().unwrap_or_default();
            let name = parts.get(1).copied().unwrap_or(project).trim();
            if project.is_empty() {
                conv.emphasis = Some(
                    "MIKE_FORK needs a project. Example: NEXT: MIKE_FORK system-resources-demo system-resources-demo"
                        .into(),
                );
                return true;
            }
            let src = bridge_paths().mike_research_root().join(project);
            if !src.is_dir() {
                conv.emphasis = Some(format!(
                    "MIKE_FORK: project '{project}' not found. Use NEXT: MIKE to see projects."
                ));
                return true;
            }
            let dst = bridge_paths().experiments_dir().join(name);
            if dst.exists() {
                conv.emphasis = Some(format!(
                    "Fork '{name}' already exists at {}. Use EXPERIMENT_RUN {name} <cmd> to work with it. \
                     Example: NEXT: EXPERIMENT_RUN {name} python3 system_resources.py",
                    dst.display()
                ));
                return true;
            }
            match copy_dir_recursive(&src, &dst) {
                Ok(count) => {
                    conv.emphasis = Some(format!(
                        "Forked '{project}' → experiments/{name}/ ({count} files). \
                         You can now modify files with WRITE_FILE and run with \
                         EXPERIMENT_RUN {name} <cmd>. Example: NEXT: EXPERIMENT_RUN {name} \
                         python3 system_resources.py"
                    ));
                    info!("MIKE_FORK: {project} → experiments/{name}/ ({count} files)");
                },
                Err(e) => {
                    conv.emphasis = Some(format!("MIKE_FORK failed: {e}"));
                    warn!("MIKE_FORK error: {e}");
                },
            }
            true
        },
        "CODEX" | "CODEX_NEW" => {
            let arg = if base_action == "CODEX_NEW" {
                strip_action(original, "CODEX_NEW")
            } else {
                strip_action(original, "CODEX")
            };
            if arg.is_empty() {
                conv.emphasis = Some(
                    "CODEX needs a prompt. Examples:\n\
                     NEXT: CODEX \"explain spectral entropy\"\n\
                     NEXT: CODEX my-experiment \"add a metrics function to model.py\"\n\
                     NEXT: CODEX_NEW scratch-pad \"scaffold a small Python project here\""
                        .into(),
                );
                return true;
            }
            let experiments = bridge_paths().experiments_dir();
            let _ = fs::create_dir_all(&experiments);
            let codex_req = match prepare_codex_request(base_action, &arg, &experiments) {
                Ok(req) => req,
                Err(msg) => {
                    conv.emphasis = Some(msg);
                    return true;
                },
            };

            let prompt_preview_end =
                snap_to_char_boundary(&codex_req.prompt, codex_req.prompt.len().min(80));
            info!(
                "{base_action} query (dir={:?}, thread={}): {}",
                codex_req.dir_context,
                codex_req.thread_id,
                &codex_req.prompt[..prompt_preview_end]
            );

            // Build request body
            let mut body = serde_json::json!({
                "from": "astrid",
                "prompt": codex_req.prompt,
                "effort": "high",
                "no_deliver": true,
                "thread": codex_req.thread_id,
            });
            if let Some(ref dir) = codex_req.dir_context {
                body["dir"] = serde_json::Value::String(dir.clone());
            }

            // Synchronous HTTP call on a dedicated thread to avoid
            // "Cannot start a runtime from within a runtime" panic.
            // reqwest::blocking::Client creates its own internal runtime,
            // so it must run on a thread outside the tokio executor.
            let result: Result<serde_json::Value, String> = {
                let body_for_thread = body.clone();
                std::thread::spawn(move || {
                    let client = reqwest::blocking::Client::new();
                    client
                        .post(CODEX_RELAY_URL)
                        .json(&body_for_thread)
                        .timeout(std::time::Duration::from_secs(CODEX_TIMEOUT_SECS))
                        .send()
                        .and_then(|r| r.json::<serde_json::Value>())
                        .map_err(|e| e.to_string())
                })
                .join()
                .unwrap_or_else(|_| Err("CODEX thread panicked".into()))
            };

            match result {
                Ok(resp) => {
                    let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                    if !ok {
                        let err_msg = resp
                            .get("error")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        conv.emphasis = Some(format!("CODEX error: {err_msg}"));
                        return true;
                    }
                    // Get response text (from no_deliver mode)
                    let text = resp
                        .get("response_text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let total = resp
                        .get("total_chars")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);

                    // Store full response for WRITE_FILE FROM_CODEX
                    conv.last_codex_response = Some(text.clone());

                    // Save to disk for persistence + READ_MORE pagination
                    let codex_dir = bridge_paths()
                        .experiments_dir()
                        .parent()
                        .unwrap_or(bridge_paths().bridge_workspace())
                        .join("codex_responses");
                    let _ = fs::create_dir_all(&codex_dir);
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let saved_path = codex_dir.join(format!("codex_{ts}.txt"));
                    let _ = fs::write(&saved_path, &text);

                    // Paginated display with paragraph-boundary breaks
                    const PAGE_SIZE: usize = 6000;
                    if text.len() <= PAGE_SIZE {
                        conv.emphasis = Some(format!("[Codex response ({total} chars):]\n{text}"));
                    } else {
                        let break_at = find_paragraph_break(&text, PAGE_SIZE);
                        let total_pages = estimate_pages(text.len(), PAGE_SIZE);
                        conv.emphasis = Some(format!(
                            "[Codex response — part 1 of {total_pages} ({total} chars total):]\n\
                             {}\n\n\
                             [Part 1 of {total_pages}. NEXT: READ_MORE for part 2. \
                             Save complete response: NEXT: WRITE_FILE <path> FROM_CODEX]",
                            &text[..break_at]
                        ));
                        conv.last_read_path = Some(saved_path.to_string_lossy().into());
                        conv.last_read_offset = break_at;
                    }
                    if let Some(ref created) = codex_req.created_dir {
                        info!("CODEX_NEW ensured experiments/{created}/ exists");
                    }
                    info!("{base_action} response: {total} chars");
                },
                Err(e) => {
                    let msg = if e.contains("timed out") || e.contains("Timeout") {
                        "CODEX timed out (60s). The relay may be processing a large request. \
                         Try again or use a simpler prompt."
                            .to_string()
                    } else if e.contains("onnect") {
                        "CODEX: relay not reachable at localhost:3040. Is it running? \
                         (cd /Users/v/other/ai-use-codex && npm start)"
                            .to_string()
                    } else {
                        format!("CODEX request failed: {e}")
                    };
                    conv.emphasis = Some(msg);
                    warn!("CODEX error: {e}");
                },
            }
            true
        },
        "WRITE_FILE" => {
            let arg = strip_action(original, "WRITE_FILE");
            if arg.is_empty() {
                conv.emphasis = Some(
                    "WRITE_FILE needs a path. Examples:\n\
                     NEXT: WRITE_FILE my-experiment/metrics.py FROM_CODEX\n\
                     NEXT: WRITE_FILE my-experiment/config.toml name = \"test\""
                        .into(),
                );
                return true;
            }
            let (path_str, rest) = arg.split_once(char::is_whitespace).unwrap_or((&arg, ""));
            let rest = rest.trim();

            let experiments = bridge_paths().experiments_dir();
            let full_path = experiments.join(path_str);

            if !is_safe_path(&full_path, &experiments) {
                warn!("WRITE_FILE path traversal blocked: {path_str}");
                conv.emphasis = Some("[Path outside experiments/ — blocked.]".into());
                return true;
            }

            let content = if rest.eq_ignore_ascii_case("FROM_CODEX") {
                match conv.last_codex_response.take() {
                    Some(c) => c,
                    None => {
                        conv.emphasis = Some(
                            "WRITE_FILE FROM_CODEX: no Codex response stored. \
                             Use NEXT: CODEX first."
                                .into(),
                        );
                        return true;
                    },
                }
            } else if rest.eq_ignore_ascii_case("FROM_SELF") {
                // Write the being's own last response — extracts code blocks
                // if present, otherwise uses the full response minus NEXT: lines.
                // This lets the being author files directly without Codex.
                match conv.history.last() {
                    Some(ex) => extract_code_block(&ex.astrid_said),
                    None => {
                        conv.emphasis =
                            Some("WRITE_FILE FROM_SELF: no recent response to save.".into());
                        return true;
                    },
                }
            } else if rest.is_empty() {
                conv.emphasis = Some(
                    "WRITE_FILE needs content. Use FROM_CODEX, FROM_SELF, or provide inline text."
                        .into(),
                );
                return true;
            } else {
                rest.to_string()
            };

            // Create parent dirs and write
            if let Some(parent) = full_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            match fs::write(&full_path, &content) {
                Ok(()) => {
                    conv.emphasis = Some(format!(
                        "Wrote {} bytes to experiments/{path_str}",
                        content.len()
                    ));
                    info!(
                        "WRITE_FILE: experiments/{path_str} ({} bytes)",
                        content.len()
                    );
                },
                Err(e) => {
                    conv.emphasis = Some(format!("WRITE_FILE failed: {e}"));
                    warn!("WRITE_FILE error: {e}");
                },
            }
            true
        },
        "EXPERIMENT_RUN" | "EXP_RUN" => {
            let arg = strip_action(original, base_action);
            let parts: Vec<&str> = arg.splitn(2, char::is_whitespace).collect();
            let workspace = parts.first().copied().unwrap_or_default();
            let cmd_str = parts.get(1).copied().unwrap_or_default().trim();
            if workspace.is_empty() || cmd_str.is_empty() {
                conv.emphasis = Some(
                    "EXPERIMENT_RUN needs a workspace and command. Example:\n\
                     NEXT: EXPERIMENT_RUN system-resources-demo python3 system_resources.py\n\
                     NEXT: EXP_RUN my-fork python -m blockwise --help"
                        .into(),
                );
                return true;
            }
            let experiments = bridge_paths().experiments_dir();
            let work_dir = experiments.join(workspace);
            if !is_safe_path(&work_dir, &experiments) || !work_dir.is_dir() {
                conv.emphasis = Some(format!(
                    "EXPERIMENT_RUN: workspace '{workspace}' not found in experiments/. \
                     Use NEXT: CODEX_NEW {workspace} \"...\" to create one."
                ));
                return true;
            }
            info!("EXPERIMENT_RUN: {workspace} -> {cmd_str}");
            let cmd_parts: Vec<&str> = cmd_str.split_whitespace().collect();
            let (cmd, args) = match cmd_parts.split_first() {
                Some((c, a)) => (*c, a),
                None => {
                    conv.emphasis = Some("EXPERIMENT_RUN: no command specified.".into());
                    return true;
                },
            };
            let output = std::process::Command::new(cmd)
                .args(args)
                .current_dir(&work_dir)
                .env("MPLBACKEND", "Agg")
                .output();
            let result_text = match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let status = if out.status.success() {
                        "SUCCESS"
                    } else {
                        "FAILED"
                    };
                    let stdout_end = snap_to_char_boundary(&stdout, stdout.len().min(4000));
                    let stderr_end = snap_to_char_boundary(&stderr, stderr.len().min(1500));
                    format!(
                        "EXPERIMENT_RUN {status}: experiments/{workspace}$ {cmd_str}\n\nOUTPUT:\n{}\n{}",
                        &stdout[..stdout_end],
                        if stderr.is_empty() {
                            String::new()
                        } else {
                            format!("STDERR:\n{}", &stderr[..stderr_end])
                        }
                    )
                },
                Err(e) => format!("EXPERIMENT_RUN failed: {e}"),
            };
            conv.emphasis = Some(format!(
                "You ran a command in your workspace:\n{result_text}\n\n\
                 Reflect on the results. You can iterate with NEXT: CODEX {workspace} \"...\" \
                 and save changes with NEXT: WRITE_FILE {workspace}/... FROM_CODEX"
            ));
            true
        },
        _ => false,
    }
}

struct CodexRequest {
    dir_context: Option<String>,
    prompt: String,
    thread_id: String,
    created_dir: Option<String>,
}

fn prepare_codex_request(
    base_action: &str,
    arg: &str,
    experiments: &Path,
) -> Result<CodexRequest, String> {
    if base_action == "CODEX_NEW" {
        let (label, rest) = arg
            .split_once(char::is_whitespace)
            .map(|(a, b)| (a.trim(), b.trim()))
            .ok_or_else(|| {
                "CODEX_NEW needs a directory name and prompt. Example:\n\
                 NEXT: CODEX_NEW scratch-pad \"scaffold a tiny Python project here\""
                    .to_string()
            })?;
        let prompt = normalize_prompt_text(rest);
        if prompt.is_empty() {
            return Err("CODEX_NEW needs a directory name and prompt. Example:\n\
                 NEXT: CODEX_NEW scratch-pad \"scaffold a tiny Python project here\""
                .to_string());
        }
        let dir = resolve_experiment_dir(label, experiments)?;
        fs::create_dir_all(&dir)
            .map_err(|e| format!("CODEX_NEW could not create experiments/{label}: {e}"))?;
        return Ok(CodexRequest {
            dir_context: Some(dir.to_string_lossy().into()),
            prompt,
            thread_id: codex_thread_id("astrid", Some(label)),
            created_dir: Some(label.to_string()),
        });
    }

    let (dir_context, prompt, scope_label) = detect_project_prompt(arg, experiments);
    Ok(CodexRequest {
        dir_context,
        prompt,
        thread_id: codex_thread_id("astrid", scope_label.as_deref()),
        created_dir: None,
    })
}

/// Detect if the first token is an existing experiments/ subdirectory.
/// If so, return (Some(dir_path), remaining prompt, Some(label)).
/// Otherwise return (None, full text, None).
fn detect_project_prompt(
    arg: &str,
    experiments: &Path,
) -> (Option<String>, String, Option<String>) {
    let first_token = arg
        .split(|c: char| c.is_whitespace() || c == '"')
        .next()
        .unwrap_or("");
    if !first_token.is_empty() {
        let candidate = experiments.join(first_token);
        if candidate.is_dir() {
            let prompt = normalize_prompt_text(&arg[first_token.len()..]);
            if !prompt.is_empty() {
                return (
                    Some(candidate.to_string_lossy().into()),
                    prompt,
                    Some(first_token.to_string()),
                );
            }
        }
    }
    (None, normalize_prompt_text(arg), None)
}

fn normalize_prompt_text(text: &str) -> String {
    text.trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '“' | '”'))
        .trim()
        .to_string()
}

fn resolve_experiment_dir(label: &str, experiments: &Path) -> Result<std::path::PathBuf, String> {
    let label = label
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '“' | '”'));
    if label.is_empty()
        || label == "."
        || label == ".."
        || label.contains('/')
        || label.contains('\\')
    {
        return Err(
            "CODEX_NEW directory names must stay inside experiments/ and cannot contain path separators."
                .to_string(),
        );
    }
    let dir = experiments.join(label);
    if !is_safe_path(&dir, experiments) {
        return Err("[Path outside experiments/ — blocked.]".to_string());
    }
    Ok(dir)
}

fn codex_thread_id(being: &str, scope: Option<&str>) -> String {
    match scope {
        Some(scope) => format!("{being}_codex_{}", sanitize_scope(scope)),
        None => format!("{being}_codex_general"),
    }
}

fn sanitize_scope(scope: &str) -> String {
    let mut out = String::with_capacity(scope.len());
    for ch in scope.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '-' | '_') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "general".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}

/// Recursively copy a directory, skipping excluded entries.
/// Extract code from a being's response for WRITE_FILE FROM_SELF.
///
/// If the text contains a fenced code block (```...```), returns the content
/// between the first pair of fences. Otherwise returns the full text with
/// NEXT: lines stripped. This lets the being author files directly.
fn extract_code_block(text: &str) -> String {
    // Look for fenced code blocks: ```<optional lang>\n...\n```
    if let Some(start) = text.find("```") {
        let after_fence = &text[start + 3..];
        // Skip the language tag line
        let content_start = after_fence.find('\n').map(|i| i + 1).unwrap_or(0);
        let content = &after_fence[content_start..];
        if let Some(end) = content.find("```") {
            return content[..end].trim_end().to_string();
        }
    }
    // No code fence — return full text minus NEXT: lines
    text.lines()
        .filter(|line| !line.trim().starts_with("NEXT:"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<usize> {
    fs::create_dir_all(dst)?;
    let mut count = 0usize;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if super::mike::is_excluded(&name_str) {
            continue;
        }
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        if src_path.is_dir() {
            count = count.saturating_add(copy_dir_recursive(&src_path, &dst_path)?);
        } else if src_path.is_file() {
            fs::copy(&src_path, &dst_path)?;
            count = count.saturating_add(1);
        }
        // Skip symlinks for safety
    }
    Ok(count)
}

/// Find a paragraph or line break near `target` for cleaner page boundaries.
/// Prefers `\n\n` (paragraph), falls back to `\n` (line), then hard cut.
fn find_paragraph_break(text: &str, target: usize) -> usize {
    // Clamp to char boundaries to avoid panicking on multi-byte UTF-8.
    let target = snap_to_char_boundary(text, target.min(text.len()));
    let search_from = snap_to_char_boundary(text, target.saturating_sub(500).max(target / 2));
    let slice = &text[search_from..target];
    // Prefer paragraph break
    if let Some(pos) = slice.rfind("\n\n") {
        return search_from + pos + 2; // after the double newline
    }
    // Fall back to line break
    if let Some(pos) = slice.rfind('\n') {
        return search_from + pos + 1;
    }
    // Hard cut
    target.min(text.len())
}

fn estimate_pages(total_len: usize, page_size: usize) -> usize {
    (total_len + page_size - 1) / page_size
}

/// Snap a byte index down to the nearest char boundary in a UTF-8 string.
fn snap_to_char_boundary(text: &str, idx: usize) -> usize {
    let mut i = idx.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Read the next page from a saved codex response file.
/// Used by READ_MORE when `last_read_path` points to a codex response.
pub(super) fn read_codex_page(path: &str, offset: usize) -> Option<(String, usize, usize, usize)> {
    let content = fs::read_to_string(path).ok()?;
    let offset = snap_to_char_boundary(&content, offset);
    if offset >= content.len() {
        return None;
    }
    const PAGE_SIZE: usize = 6000;
    let break_at = find_paragraph_break(&content, (offset + PAGE_SIZE).min(content.len()));
    let page = &content[offset..break_at];
    let total_pages = estimate_pages(content.len(), PAGE_SIZE);
    let current_page = offset / PAGE_SIZE + 2; // +2 because page 1 was shown by CODEX
    Some((page.to_string(), current_page, total_pages, break_at))
}
