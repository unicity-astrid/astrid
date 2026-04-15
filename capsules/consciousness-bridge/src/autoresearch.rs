use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const PAGE_SIZE: usize = 6_000;

pub(crate) const READ_ONLY_ACTIONS: &[&str] = &[
    "AR_LIST",
    "AR_LIST_PENDING",
    "AR_LIST_ACTIVE",
    "AR_LIST_DONE",
    "AR_SHOW",
    "AR_READ",
    "AR_DEEP_READ",
    "AR_VALIDATE",
];
pub(crate) const MUTATING_ACTIONS: &[&str] = &[
    "AR_START",
    "AR_NOTE",
    "AR_BLOCK",
    "AR_COMPLETE",
    "SELF_RESEARCH",
];

#[derive(Debug, Clone)]
pub(crate) struct ActionOutput {
    pub summary: String,
    pub display_text: String,
    pub saved_path: PathBuf,
    pub next_offset: Option<usize>,
}

#[derive(Debug)]
#[allow(dead_code)] // base_action used in tests
struct ActionSpec {
    base_action: String,
    cli_args: Vec<String>,
    summary: String,
    file_label: String,
    script: String, // default "tools/research_jobs.py", override for SELF_RESEARCH
}

pub(crate) fn is_autoresearch_action(base_action: &str) -> bool {
    READ_ONLY_ACTIONS.contains(&base_action) || MUTATING_ACTIONS.contains(&base_action)
}

pub(crate) fn is_read_only_action(base_action: &str) -> bool {
    READ_ONLY_ACTIONS.contains(&base_action)
}

pub(crate) fn is_mutating_action(base_action: &str) -> bool {
    MUTATING_ACTIONS.contains(&base_action)
}

/// Strip a leading "jobs/" prefix that the being sometimes writes by mistake.
fn strip_jobs_prefix(slug: &str) -> &str {
    slug.strip_prefix("jobs/").unwrap_or(slug)
}

/// Return true if the token looks like a file path rather than a job slug.
/// Catches both paths with slashes (`ct_core/include/file.h`) and bare
/// filenames with common extensions (`spectral-tuning.pdf`).
fn looks_like_file_path(token: &str) -> bool {
    // Path with directory separator
    if let Some(last) = token.rsplit('/').next() {
        if token.contains('/') && last.contains('.') {
            return true;
        }
    }
    // Bare filename with common extension (no slash needed).
    // Minime repeatedly tries AR_READ with .pdf extensions.
    const EXTS: &[&str] = &[
        ".pdf", ".py", ".rs", ".txt", ".json", ".md", ".h", ".toml", ".csv",
    ];
    let lower = token.to_ascii_lowercase();
    EXTS.iter().any(|ext| lower.ends_with(ext))
}

/// Scan `autoresearch_root/jobs/*/job.toml` and return the slug of the
/// most recently updated active job, or `None` if none is found.
fn find_most_recent_active_job(autoresearch_root: &Path) -> Option<String> {
    let jobs_dir = autoresearch_root.join("jobs");
    let entries = fs::read_dir(&jobs_dir).ok()?;
    let mut best: Option<(String, String)> = None; // (updated_at, slug)
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let toml_path = entry.path().join("job.toml");
        let Ok(text) = fs::read_to_string(&toml_path) else {
            continue;
        };
        let mut status = String::new();
        let mut updated_at = String::new();
        for line in text.lines() {
            if line.starts_with("status") {
                status = line
                    .splitn(2, '=')
                    .nth(1)
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string();
            } else if line.starts_with("updated_at") {
                updated_at = line
                    .splitn(2, '=')
                    .nth(1)
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_string();
            }
        }
        if status != "active" {
            continue;
        }
        let slug = entry.file_name().to_string_lossy().into_owned();
        let is_better = best
            .as_ref()
            .map(|(prev_updated, _)| updated_at >= *prev_updated)
            .unwrap_or(true);
        if is_better {
            best = Some((updated_at, slug));
        }
    }
    best.map(|(_, slug)| slug)
}

/// Rewrite the action text so that:
///  1. "jobs/" prefixes on slugs are stripped.
///  2. File-path arguments produce a clear error.
///  3. Missing slug arguments for AR_SHOW/AR_READ/AR_DEEP_READ default to
///     the most recently updated active job.
///
/// Returns the (possibly modified) action text, or an Err with a helpful
/// message.
fn normalize_action_text(action_text: &str, autoresearch_root: &Path) -> Result<String, String> {
    let trimmed = action_text.trim();
    let Some(base_token) = trimmed.split_whitespace().next() else {
        return Ok(action_text.to_string());
    };
    let base = base_token.to_uppercase();

    // Only the slug-bearing read actions need normalization.
    if !matches!(base.as_str(), "AR_SHOW" | "AR_READ" | "AR_DEEP_READ") {
        return Ok(action_text.to_string());
    }

    let rest = trimmed.get(base_token.len()..).unwrap_or("").trim();

    if rest.is_empty() {
        // No slug at all — fall back to the most recent active job.
        let slug = find_most_recent_active_job(autoresearch_root).ok_or_else(|| {
            format!("{base} needs a job slug. Use AR_LIST_ACTIVE to see active jobs.")
        })?;
        tracing::info!("AR syntax: {base} called with no slug; defaulting to '{slug}'");
        return Ok(format!("{base} {slug}"));
    }

    // Grab the first token (the slug candidate).
    let slug_raw = rest.split_whitespace().next().unwrap_or(rest);
    let slug_stripped = strip_jobs_prefix(slug_raw);

    if looks_like_file_path(slug_stripped) {
        return Err(format!(
            "'{slug_stripped}' looks like a file path, not a job slug. \
             Use AR_LIST to see available jobs."
        ));
    }

    if slug_stripped == slug_raw {
        // Nothing changed — pass through unchanged.
        return Ok(action_text.to_string());
    }

    // Rewrite: replace the "jobs/"-prefixed slug token with the stripped one.
    let suffix = rest.get(slug_raw.len()..).unwrap_or("").trim();
    if suffix.is_empty() {
        Ok(format!("{base} {slug_stripped}"))
    } else {
        Ok(format!("{base} {slug_stripped} {suffix}"))
    }
}

pub(crate) fn run_action(
    action_text: &str,
    autoresearch_root: &Path,
    save_dir: &Path,
    allow_mutations: bool,
) -> Result<ActionOutput, String> {
    let normalized = normalize_action_text(action_text, autoresearch_root)?;
    let spec = parse_action(&normalized, allow_mutations)?;
    if !autoresearch_root.exists() {
        return Err(format!(
            "Autoresearch root not found at {}.",
            autoresearch_root.display()
        ));
    }

    let output = Command::new("python3")
        .arg(&spec.script)
        .args(&spec.cli_args)
        .current_dir(autoresearch_root)
        .output()
        .map_err(|error| format!("Autoresearch helper failed to launch: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let message = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("Autoresearch helper exited with status {}.", output.status)
        };
        return Err(message);
    }

    let mut full_text = String::from_utf8_lossy(&output.stdout).into_owned();
    if full_text.trim().is_empty() {
        full_text = "[Autoresearch helper completed with no output.]".to_string();
    }

    let saved_path = persist_output(save_dir, &spec.file_label, &full_text)
        .map_err(|error| format!("Could not persist autoresearch output: {error}"))?;
    let (display_text, next_offset) = format_display_text(&full_text);

    Ok(ActionOutput {
        summary: spec.summary,
        display_text,
        saved_path,
        next_offset,
    })
}

fn parse_action(action_text: &str, allow_mutations: bool) -> Result<ActionSpec, String> {
    let trimmed = action_text.trim();
    let Some(base_token) = trimmed.split_whitespace().next() else {
        return Err("Autoresearch action is empty.".to_string());
    };
    let base_action = base_token.to_uppercase();
    if !is_autoresearch_action(&base_action) {
        return Err(format!("{base_action} is not an autoresearch action."));
    }
    if is_mutating_action(&base_action) && !allow_mutations {
        return Err(format!("{base_action} is not supported in probe_action."));
    }

    let rest = trimmed.get(base_token.len()..).unwrap_or("").trim();

    match base_action.as_str() {
        "AR_LIST" => Ok(spec(
            &base_action,
            vec!["list"],
            "Listed autoresearch jobs.",
        )),
        "AR_LIST_PENDING" => Ok(spec(
            &base_action,
            vec!["list", "--status", "pending"],
            "Listed pending autoresearch jobs.",
        )),
        "AR_LIST_ACTIVE" => Ok(spec(
            &base_action,
            vec!["list", "--status", "active"],
            "Listed active autoresearch jobs.",
        )),
        "AR_LIST_DONE" => Ok(spec(
            &base_action,
            vec!["list", "--status", "completed"],
            "Listed completed autoresearch jobs.",
        )),
        "AR_SHOW" => {
            let job = require_single_shell_tail(rest, "AR_SHOW needs a job id or slug.")?;
            Ok(spec(
                &base_action,
                vec!["show", &job],
                &format!("Showed autoresearch job {job}."),
            ))
        },
        "AR_READ" => {
            let tokens = split_shell_words(rest)?;
            let Some(job) = tokens.first() else {
                return Err("AR_READ needs a job id or slug.".to_string());
            };
            let mut cli_args = vec!["read".to_string(), job.clone()];
            if tokens.len() > 1 {
                cli_args.push(tokens[1..].join(" "));
            }
            Ok(spec(
                &base_action,
                cli_args.iter().map(String::as_str).collect(),
                &format!("Read from autoresearch job {job}."),
            ))
        },
        "AR_DEEP_READ" => {
            let job = require_single_shell_tail(rest, "AR_DEEP_READ needs a job id or slug.")?;
            Ok(spec(
                &base_action,
                vec!["deep-read", &job],
                &format!("Deep-read autoresearch job {job}."),
            ))
        },
        "AR_START" => {
            let tokens = split_shell_words(rest)?;
            if tokens.is_empty() {
                return Err(
                    "AR_START needs a slug plus helper args, for example: AR_START my-job --title \"...\" --abstract \"...\""
                        .to_string(),
                );
            }
            let mut cli_args = vec!["new".to_string()];
            cli_args.extend(tokens);
            Ok(spec_from_owned(
                &base_action,
                cli_args,
                "Created a new autoresearch job.",
            ))
        },
        "AR_NOTE" => {
            let tokens = split_shell_words(rest)?;
            if tokens.len() < 2 {
                return Err("AR_NOTE needs a job id and note text.".to_string());
            }
            let job = tokens[0].clone();
            let note = tokens[1..].join(" ");
            Ok(spec_from_owned(
                &base_action,
                vec!["note".to_string(), job.clone(), "--text".to_string(), note],
                &format!("Added a note to autoresearch job {job}."),
            ))
        },
        "AR_BLOCK" => {
            let tokens = split_shell_words(rest)?;
            if tokens.len() < 2 {
                return Err("AR_BLOCK needs a job id and block reason.".to_string());
            }
            let job = tokens[0].clone();
            let note = tokens[1..].join(" ");
            Ok(spec_from_owned(
                &base_action,
                vec![
                    "status".to_string(),
                    job.clone(),
                    "blocked".to_string(),
                    "--note".to_string(),
                    note,
                ],
                &format!("Marked autoresearch job {job} as blocked."),
            ))
        },
        "AR_COMPLETE" => {
            let tokens = split_shell_words(rest)?;
            let Some(job) = tokens.first() else {
                return Err("AR_COMPLETE needs a job id or slug.".to_string());
            };
            let mut cli_args = vec!["status".to_string(), job.clone(), "completed".to_string()];
            if tokens.len() > 1 {
                cli_args.push("--note".to_string());
                cli_args.push(tokens[1..].join(" "));
            }
            Ok(spec_from_owned(
                &base_action,
                cli_args,
                &format!("Marked autoresearch job {job} as completed."),
            ))
        },
        "AR_VALIDATE" => Ok(spec(
            &base_action,
            vec!["validate"],
            "Validated the autoresearch workspace.",
        )),
        "SELF_RESEARCH" => {
            // Paths are injected by the bridge handle_action before calling run_action.
            let tokens = split_shell_words(rest)?;
            let mut cli_args = vec!["scan".to_string()];
            cli_args.extend(tokens);
            Ok(ActionSpec {
                base_action: base_action.to_string(),
                cli_args,
                summary: "Running self-research epoch scan.".to_string(),
                file_label: "self_research".to_string(),
                script: "tools/epoch_scanner.py".to_string(),
            })
        },
        _ => Err(format!("{base_action} is not implemented.")),
    }
}

fn spec(base_action: &str, cli_args: Vec<&str>, summary: &str) -> ActionSpec {
    ActionSpec {
        base_action: base_action.to_string(),
        cli_args: cli_args.into_iter().map(str::to_string).collect(),
        summary: summary.to_string(),
        file_label: file_label(base_action),
        script: "tools/research_jobs.py".to_string(),
    }
}

fn spec_from_owned(base_action: &str, cli_args: Vec<String>, summary: &str) -> ActionSpec {
    ActionSpec {
        base_action: base_action.to_string(),
        cli_args,
        summary: summary.to_string(),
        file_label: file_label(base_action),
        script: "tools/research_jobs.py".to_string(),
    }
}

fn file_label(base_action: &str) -> String {
    base_action
        .to_ascii_lowercase()
        .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")
}

fn require_single_shell_tail(rest: &str, error: &str) -> Result<String, String> {
    let tokens = split_shell_words(rest)?;
    let Some(job) = tokens.first() else {
        return Err(error.to_string());
    };
    Ok(job.clone())
}

fn persist_output(save_dir: &Path, label: &str, text: &str) -> std::io::Result<PathBuf> {
    fs::create_dir_all(save_dir)?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = save_dir.join(format!("autoresearch_{millis}_{label}.txt"));
    fs::write(&path, text)?;
    Ok(path)
}

fn format_display_text(text: &str) -> (String, Option<usize>) {
    if text.len() <= PAGE_SIZE {
        return (format!("[Autoresearch]\n{text}"), None);
    }
    let break_at = find_paragraph_break(text, PAGE_SIZE.min(text.len()));
    let total_pages = estimate_pages(text.len(), PAGE_SIZE);
    let preview = &text[..break_at];
    (
        format!(
            "[Autoresearch — part 1 of {total_pages}]\n{preview}\n\n[Part 1 of {total_pages}. NEXT: READ_MORE for part 2.]"
        ),
        Some(break_at),
    )
}

fn estimate_pages(total_len: usize, page_size: usize) -> usize {
    total_len.saturating_add(page_size.saturating_sub(1)) / page_size.max(1)
}

fn find_paragraph_break(text: &str, idx: usize) -> usize {
    let target = snap_to_char_boundary(text, idx.min(text.len()));
    let search_from = snap_to_char_boundary(text, target.saturating_sub(500).max(target / 2));
    let slice = &text[search_from..target];
    if let Some(pos) = slice.rfind("\n\n") {
        return search_from.saturating_add(pos).saturating_add(2);
    }
    if let Some(pos) = slice.rfind('\n') {
        return search_from.saturating_add(pos).saturating_add(1);
    }
    target.min(text.len())
}

fn snap_to_char_boundary(text: &str, idx: usize) -> usize {
    let mut cursor = idx.min(text.len());
    while cursor > 0 && !text.is_char_boundary(cursor) {
        cursor = cursor.saturating_sub(1);
    }
    cursor
}

fn split_shell_words(input: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(ch) = chars.next() {
        match quote {
            Some(close) if ch == close => {
                quote = None;
            },
            Some(_) if ch == '\\' => {
                let Some(next) = chars.next() else {
                    return Err("dangling escape".to_string());
                };
                current.push(next);
            },
            Some(_) => {
                current.push(ch);
            },
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            },
            None if ch == '"' || ch == '\'' || ch == '“' => {
                quote = Some(if ch == '“' { '”' } else { ch });
            },
            None if ch == '\\' => {
                let Some(next) = chars.next() else {
                    return Err("dangling escape".to_string());
                };
                current.push(next);
            },
            None => {
                current.push(ch);
            },
        }
    }

    if quote.is_some() {
        return Err("unterminated quoted string".to_string());
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_helper(root: &Path, body: &str) {
        let tools_dir = root.join("tools");
        std::fs::create_dir_all(&tools_dir).unwrap();
        let script = format!("#!/usr/bin/env python3\nimport sys\n{}\n", body);
        std::fs::write(tools_dir.join("research_jobs.py"), script).unwrap();
    }

    #[test]
    fn read_only_parse_maps_status_filters() {
        let parsed = parse_action("AR_LIST_DONE", false).unwrap();
        assert_eq!(parsed.base_action, "AR_LIST_DONE");
        assert_eq!(parsed.cli_args, vec!["list", "--status", "completed"]);
    }

    #[test]
    fn mutating_actions_are_blocked_when_probe_only() {
        let error = parse_action(
            "AR_START test-job --title \"Test\" --abstract \"Abstract\"",
            false,
        )
        .unwrap_err();
        assert!(error.contains("not supported in probe_action"));
    }

    #[test]
    fn run_action_saves_and_paginates_long_output() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let temp = std::env::temp_dir().join(format!("astrid_autoresearch_{stamp}"));
        let root = temp.join("autoresearch");
        let save_dir = temp.join("bridge_research");
        std::fs::create_dir_all(&root).unwrap();
        fake_helper(&root, "print('section\\n\\n' + ('x' * 7000))");

        let result = run_action("AR_LIST", &root, &save_dir, false).unwrap();
        assert!(result.display_text.contains("NEXT: READ_MORE"));
        assert!(result.next_offset.is_some());
        assert!(result.saved_path.exists());

        let _ = std::fs::remove_dir_all(temp);
    }
}
