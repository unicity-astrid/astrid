use std::fs;
use std::path::Path;
use tracing::{info, warn};

use super::{ConversationState, NextActionContext, strip_action};
use crate::paths::bridge_paths;

/// Directories to exclude from MIKE listings (build artifacts, venvs).
const EXCLUDED: &[&str] = &[
    "__pycache__",
    ".venv",
    ".build",
    "node_modules",
    ".git",
    ".DS_Store",
    "target",
    ".mypy_cache",
];

pub(super) fn handle_action(
    conv: &mut ConversationState,
    base_action: &str,
    original: &str,
    _ctx: &mut NextActionContext<'_>,
) -> bool {
    match base_action {
        "MIKE" => {
            let root = bridge_paths().mike_research_root();
            let listing = mike_overview(&root);
            conv.pending_file_listing = Some(listing);
            info!("Astrid browsed MIKE curated research");
            true
        },
        "MIKE_BROWSE" => {
            let project = normalize_action_arg(&strip_action(original, "MIKE_BROWSE"));
            if project.is_empty() {
                conv.pending_file_listing = Some(
                    "[MIKE_BROWSE needs a project name. Use NEXT: MIKE to see available projects.]"
                        .into(),
                );
                return true;
            }
            let root = bridge_paths().mike_research_root();
            let project_dir = root.join(&project);
            if !is_safe_path(&project_dir, &root) {
                warn!("MIKE_BROWSE path traversal blocked: {project}");
                conv.pending_file_listing =
                    Some("[Path outside research directory — blocked.]".into());
                return true;
            }
            if !project_dir.is_dir() {
                conv.pending_file_listing = Some(format!(
                    "[Project '{project}' not found. Use NEXT: MIKE to see available projects.]"
                ));
                return true;
            }
            let listing = mike_browse_project(&project_dir, &project);
            conv.pending_file_listing = Some(listing);
            info!("Astrid browsed MIKE project: {project}");
            true
        },
        "MIKE_READ" => {
            let path_arg = normalize_action_arg(&strip_action(original, "MIKE_READ"));
            if path_arg.is_empty() {
                conv.pending_file_listing = Some(
                    "[MIKE_READ needs a path. Example: NEXT: MIKE_READ blockwise/README.md]".into(),
                );
                return true;
            }
            let root = bridge_paths().mike_research_root();
            let file_path = root.join(&path_arg);
            if !is_safe_path(&file_path, &root) {
                warn!("MIKE_READ path traversal blocked: {path_arg}");
                conv.pending_file_listing =
                    Some("[Path outside research directory — blocked.]".into());
                return true;
            }
            if !file_path.exists() {
                conv.pending_file_listing = Some(format!(
                    "[File '{path_arg}' not found in research. Use NEXT: MIKE_BROWSE <project> to see files.]"
                ));
                return true;
            }
            if file_path.is_dir() {
                let listing = mike_browse_project(&file_path, &path_arg);
                conv.pending_file_listing = Some(listing);
                conv.last_read_path = None;
                conv.last_read_offset = 0;
                conv.last_read_meaning_summary = None;
                return true;
            }
            if is_pdf_path(&file_path) {
                match super::pdf::read_pdf_window(&file_path, &root, 1, super::pdf::PDF_CHAR_BUDGET)
                {
                    Ok(window) => {
                        conv.pending_file_listing =
                            Some(super::pdf::format_initial_window(&path_arg, &window));
                        if let Some(next_page) = window.next_page {
                            conv.last_read_path = Some(super::pdf::marker_for_path(&file_path));
                            conv.last_read_offset = next_page;
                        } else {
                            conv.last_read_path = None;
                            conv.last_read_offset = 0;
                        }
                        conv.last_read_meaning_summary = None;
                        info!(
                            "Astrid read MIKE PDF: {} (pages {}-{} of {})",
                            path_arg, window.first_page, window.last_page, window.total_pages
                        );
                    },
                    Err(err) => {
                        conv.pending_file_listing = Some(err.clone());
                        conv.last_read_path = None;
                        conv.last_read_offset = 0;
                        conv.last_read_meaning_summary = None;
                        warn!("MIKE_READ PDF failed for {}: {}", path_arg, err);
                    },
                }
                return true;
            }
            if is_binary_extension(&file_path) {
                let size = file_path.metadata().map(|m| m.len()).unwrap_or(0);
                let ext = file_path.extension().unwrap_or_default().to_string_lossy();
                conv.pending_file_listing = Some(format!(
                    "[{path_arg} is a {ext} file ({} KB). Binary files can't be read as text. \
                     Try NEXT: MIKE_SEARCH to find concepts from this material, \
                     or NEXT: SEARCH to look up the topic online.]",
                    size / 1024
                ));
                return true;
            }
            let content = read_file_paginated(&file_path, conv.last_read_offset);
            conv.pending_file_listing = Some(format!("[Research file: {path_arg}]\n{content}"));
            conv.last_read_path = Some(file_path.to_string_lossy().into());
            // Advance offset for READ_MORE
            let lines_shown = content.lines().count();
            conv.last_read_offset = conv.last_read_offset.saturating_add(lines_shown);
            conv.last_read_meaning_summary = None;
            info!("Astrid read MIKE file: {path_arg}");
            true
        },
        "MIKE_SEARCH" => {
            let pattern = normalize_action_arg(&strip_action(original, "MIKE_SEARCH"));
            if pattern.is_empty() {
                conv.pending_file_listing = Some(
                    "[MIKE_SEARCH needs a pattern. Example: NEXT: MIKE_SEARCH spectral radius]"
                        .into(),
                );
                return true;
            }
            let root = bridge_paths().mike_research_root();
            let results = mike_search(&root, &pattern);
            conv.pending_file_listing = Some(results);
            info!("Astrid searched MIKE research for: {pattern}");
            true
        },
        "MIKE_RUN" => {
            let arg = strip_action(original, "MIKE_RUN");
            let tokens = match split_shell_words(&arg) {
                Ok(tokens) => tokens,
                Err(err) => {
                    conv.emphasis = Some(format!("MIKE_RUN could not parse command: {err}"));
                    return true;
                },
            };
            if tokens.len() < 2 {
                conv.emphasis = Some(
                    "MIKE_RUN needs a project and script. Example: NEXT: MIKE_RUN blockwise python -m blockwise --help"
                        .into(),
                );
                return true;
            }
            let project = tokens[0].as_str();
            let root = bridge_paths().mike_research_root();
            let project_dir = root.join(project);
            if !is_safe_path(&project_dir, &root) || !project_dir.is_dir() {
                conv.emphasis = Some(format!(
                    "MIKE_RUN: project '{project}' not found. Use NEXT: MIKE to see projects."
                ));
                return true;
            }
            let command_text = tokens[1..].join(" ");
            let (cmd, args) = match tokens[1..].split_first() {
                Some((cmd, args)) => (cmd.as_str(), args),
                None => {
                    conv.emphasis = Some("MIKE_RUN: no command specified.".into());
                    return true;
                },
            };
            info!("Astrid running MIKE script: {project} -> {command_text}");
            let output = std::process::Command::new(cmd)
                .args(args)
                .current_dir(&project_dir)
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
                    format!(
                        "MIKE_RUN {status}: {project}/{command_text}\n\nOUTPUT:\n{}\n{}",
                        &stdout[..stdout.floor_char_boundary(3000)],
                        if stderr.is_empty() {
                            String::new()
                        } else {
                            format!("STDERR:\n{}", &stderr[..stderr.floor_char_boundary(1000)])
                        }
                    )
                },
                Err(e) => format!("MIKE_RUN failed: {e}"),
            };
            conv.emphasis = Some(format!(
                "You ran an experiment in Mike's research:\n{result_text}\n\nReflect on these results."
            ));
            true
        },
        _ => false,
    }
}

/// Read MIKE_INDEX.toml and present an overview of curated projects.
fn mike_overview(root: &Path) -> String {
    let mut out = String::from(
        "[Mike's curated research — use MIKE_BROWSE <project> to explore, MIKE_READ <path> to read text files or PDFs]\n\n",
    );
    // Try MIKE_INDEX.toml for descriptions
    let index_path = root.join("MIKE_INDEX.toml");
    if let Ok(content) = fs::read_to_string(&index_path) {
        if let Some(projects_section) = content.split("[projects]").nth(1) {
            for line in projects_section.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((slug, desc)) = line.split_once('=') {
                    let slug = slug.trim();
                    let desc = desc.trim().trim_matches('"');
                    out.push_str(&format!("  {slug}/  — {desc}\n"));
                }
            }
            return out;
        }
    }
    // Fallback: list directories with README first lines
    if let Ok(entries) = fs::read_dir(root) {
        let mut dirs: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir() && !is_excluded(e.file_name().to_string_lossy().as_ref()))
            .collect();
        dirs.sort_by_key(|e| e.file_name());
        for entry in dirs {
            let name = entry.file_name();
            let readme = entry.path().join("README.md");
            let desc = if readme.exists() {
                fs::read_to_string(&readme)
                    .ok()
                    .and_then(|s| {
                        s.lines()
                            .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
                            .map(String::from)
                    })
                    .unwrap_or_default()
            } else {
                String::new()
            };
            out.push_str(&format!("  {}/  {desc}\n", name.to_string_lossy()));
        }
    }
    out
}

/// Browse a specific project: file tree + README excerpt.
fn mike_browse_project(dir: &Path, label: &str) -> String {
    let mut out = format!("[Research project: {label}]\n\n");
    // README excerpt
    let readme = dir.join("README.md");
    if readme.exists() {
        if let Ok(content) = fs::read_to_string(&readme) {
            let excerpt: String = content.lines().take(25).collect::<Vec<_>>().join("\n");
            out.push_str(&format!(
                "--- README.md (first 25 lines) ---\n{excerpt}\n---\n\n"
            ));
        }
    }
    // File listing (filtered)
    out.push_str("Files:\n");
    if let Ok(entries) = fs::read_dir(dir) {
        let mut items: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| !is_excluded(e.file_name().to_string_lossy().as_ref()))
            .collect();
        items.sort_by_key(|e| e.file_name());
        for entry in &items {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let meta = entry.metadata().ok();
            if entry.path().is_dir() {
                out.push_str(&format!("  {name_str}/\n"));
            } else {
                let size = meta.map(|m| m.len()).unwrap_or(0);
                let size_str = if size > 1_048_576 {
                    format!("{:.1} MB", size as f64 / 1_048_576.0)
                } else {
                    format!("{:.1} KB", size as f64 / 1024.0)
                };
                out.push_str(&format!("  {name_str}  ({size_str})\n"));
            }
        }
        if items.len() > 40 {
            out.push_str(&format!("  ... {} total entries\n", items.len()));
        }
    }
    out.push_str(&format!(
        "\nUse MIKE_READ {label}/<file> to read text files or PDFs, MIKE_SEARCH <pattern> to search, MIKE_RUN {label} <cmd> to run."
    ));
    out
}

/// Search across research with grep.
fn mike_search(root: &Path, pattern: &str) -> String {
    let output = std::process::Command::new("grep")
        .args([
            "-rn",
            "--include=*.py",
            "--include=*.rs",
            "--include=*.md",
            "--include=*.toml",
            "--include=*.txt",
            "--include=*.swift",
            "-i",
            pattern,
        ])
        .current_dir(root)
        .output();
    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<&str> = text.lines().take(25).collect();
            if lines.is_empty() {
                format!("[MIKE_SEARCH: no matches for '{pattern}' in research.]")
            } else {
                let truncated = if text.lines().count() > 25 {
                    format!(
                        "\n... ({} total matches, showing first 25)",
                        text.lines().count()
                    )
                } else {
                    String::new()
                };
                format!(
                    "[MIKE_SEARCH results for '{pattern}':]\n{}{truncated}\n\nUse MIKE_READ <path> to read any text file or PDF.",
                    lines.join("\n")
                )
            }
        },
        Err(e) => format!("[MIKE_SEARCH failed: {e}]"),
    }
}

/// Read a file with pagination (400 lines per page).
fn read_file_paginated(path: &Path, offset: usize) -> String {
    const PAGE_SIZE: usize = 400;
    match fs::read_to_string(path) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().collect();
            let total = lines.len();
            let start = offset.min(total);
            let end = (start + PAGE_SIZE).min(total);
            let page = lines[start..end].join("\n");
            if end < total {
                format!(
                    "{page}\n\n[Showing lines {}-{} of {}. Use NEXT: READ_MORE to continue.]",
                    start + 1,
                    end,
                    total
                )
            } else if start > 0 {
                format!("{page}\n\n[End of file ({total} lines total).]")
            } else {
                page
            }
        },
        Err(e) => format!("[Could not read file: {e}]"),
    }
}

/// Validate a path stays within a given root directory.
pub(super) fn is_safe_path(path: &Path, root: &Path) -> bool {
    match (path.canonicalize(), root.canonicalize()) {
        (Ok(resolved), Ok(root_resolved)) => resolved.starts_with(&root_resolved),
        // If path doesn't exist yet, check the parent
        (Err(_), Ok(root_resolved)) => path
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .is_some_and(|p| p.starts_with(&root_resolved)),
        _ => false,
    }
}

pub(super) fn is_excluded(name: &str) -> bool {
    EXCLUDED.iter().any(|e| name == *e) || name.starts_with('.')
}

fn is_binary_extension(path: &Path) -> bool {
    const BINARY_EXTS: &[&str] = &[
        "png",
        "jpg",
        "jpeg",
        "gif",
        "ico",
        "zip",
        "gz",
        "tar",
        "whl",
        "so",
        "dylib",
        "bin",
        "pyc",
        "pyo",
        "o",
        "a",
        "wav",
        "mp3",
        "mp4",
        "mlmodel",
        "mlmodelc",
        "mlpackage",
    ];
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| BINARY_EXTS.iter().any(|b| ext.eq_ignore_ascii_case(b)))
}

fn is_pdf_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
}

fn normalize_action_arg(arg: &str) -> String {
    let trimmed = arg.trim();
    if trimmed.len() >= 2 {
        let quote_pairs = [('"', '"'), ('\'', '\''), ('“', '”')];
        for (open, close) in quote_pairs {
            if trimmed.starts_with(open) && trimmed.ends_with(close) {
                return trimmed[open.len_utf8()..trimmed.len() - close.len_utf8()]
                    .trim()
                    .to_string();
            }
        }
    }
    trimmed.to_string()
}

fn split_shell_words(input: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(ch) = chars.next() {
        match quote {
            Some(delim) => {
                if ch == delim {
                    quote = None;
                } else if ch == '\\' && delim == '"' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                } else {
                    current.push(ch);
                }
            },
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                },
                c if c.is_whitespace() => {
                    if !current.is_empty() {
                        words.push(std::mem::take(&mut current));
                    }
                },
                _ => current.push(ch),
            },
        }
    }

    if quote.is_some() {
        return Err("unclosed quote".into());
    }
    if !current.is_empty() {
        words.push(current);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::{normalize_action_arg, split_shell_words};

    #[test]
    fn normalize_action_arg_strips_wrapping_quotes() {
        assert_eq!(
            normalize_action_arg("\"pdfs/Attention is All You Need.pdf\""),
            "pdfs/Attention is All You Need.pdf"
        );
        assert_eq!(
            normalize_action_arg("“pdfs/rudin-math-definitions.pdf”"),
            "pdfs/rudin-math-definitions.pdf"
        );
    }

    #[test]
    fn split_shell_words_preserves_quoted_segments() {
        let tokens = split_shell_words("project python tool.py \"file with spaces.pdf\"")
            .expect("quoted command should parse");
        assert_eq!(
            tokens,
            vec![
                "project".to_string(),
                "python".to_string(),
                "tool.py".to_string(),
                "file with spaces.pdf".to_string(),
            ]
        );
    }
}
