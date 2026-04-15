use std::fs;
use std::path::Path;

use tracing::{info, warn};

use super::{
    ConversationState, NextActionContext, extract_search_topic, list_directory, strip_action,
};
use crate::memory;
use crate::paths::bridge_paths;

const READ_MORE_PAGE_CHUNK: usize = 4000;

fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
    let mut safe = offset.min(text.len());
    while safe > 0 && !text.is_char_boundary(safe) {
        safe = safe.saturating_sub(1);
    }
    safe
}

fn advance_by_chars(text: &str, start: usize, char_count: usize) -> usize {
    let start = clamp_to_char_boundary(text, start);
    if char_count == 0 || start >= text.len() {
        return start;
    }
    text[start..]
        .char_indices()
        .nth(char_count)
        .map(|(index, _)| start.saturating_add(index))
        .unwrap_or(text.len())
}

fn looks_like_raw_pdf_dump(content: &str, body_start: usize) -> bool {
    let body_start = clamp_to_char_boundary(content, body_start);
    let preview = content[body_start..].chars().take(512).collect::<String>();
    preview.starts_with("%PDF-")
}

fn normalize_read_more_hint(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn score_read_more_candidate(hint: &str, candidate: &str, rank: usize) -> f32 {
    if hint.is_empty() {
        return (32usize.saturating_sub(rank)) as f32;
    }

    let hint_norm = normalize_read_more_hint(hint);
    let candidate_norm = normalize_read_more_hint(candidate);
    if hint_norm.is_empty() || candidate_norm.is_empty() {
        return 0.0;
    }

    let mut score = (24usize.saturating_sub(rank)) as f32;
    if candidate_norm.contains(&hint_norm) || hint_norm.contains(&candidate_norm) {
        score += 80.0;
    }

    let hint_tokens = hint_norm
        .split_whitespace()
        .filter(|token| token.len() >= 3)
        .collect::<Vec<_>>();
    let candidate_tokens = candidate_norm
        .split_whitespace()
        .filter(|token| token.len() >= 3)
        .collect::<Vec<_>>();
    let overlap = hint_tokens
        .iter()
        .filter(|token| candidate_tokens.contains(token))
        .count();
    score + (overlap as f32 * 18.0)
}

fn parse_saved_page_header(content: &str) -> (usize, Option<String>) {
    let header_end = content.find("\n\n").unwrap_or(0);
    if header_end == 0 {
        return (0, None);
    }
    let header = &content[..header_end];
    let url = header.lines().find_map(|line| {
        line.strip_prefix("URL:")
            .map(|value| value.trim().to_string())
    });
    (header_end.saturating_add(2), url)
}

fn is_codex_saved_response(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name.starts_with("codex_") {
        return true;
    }
    path.components()
        .any(|component| component.as_os_str() == "codex_responses")
}

fn recover_read_more_target(
    conv: &ConversationState,
    hint: &str,
) -> Option<(String, usize, String)> {
    let mut candidates: Vec<(String, usize, String, bool)> = Vec::new();

    if let Some(path) = conv.last_read_path.clone() {
        if Path::new(&path).exists() {
            candidates.push((
                path.clone(),
                conv.last_read_offset,
                path,
                conv.last_read_offset > 0,
            ));
        }
    }

    let research_dir = bridge_paths().research_dir();
    if let Ok(entries) = fs::read_dir(&research_dir) {
        let mut page_files = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("page_") && name.ends_with(".txt"))
            })
            .collect::<Vec<_>>();
        page_files.sort_by_key(|path| fs::metadata(path).and_then(|meta| meta.modified()).ok());
        page_files.reverse();

        for page_path in page_files.into_iter().take(8) {
            let Ok(content) = fs::read_to_string(&page_path) else {
                continue;
            };
            let (header_len, url) = parse_saved_page_header(&content);
            if looks_like_raw_pdf_dump(&content, header_len) {
                continue;
            }
            let body_start = clamp_to_char_boundary(&content, header_len);
            let body_len = content[body_start..].chars().count();
            let offset = advance_by_chars(&content, body_start, READ_MORE_PAGE_CHUNK.min(body_len));
            let label = match url {
                Some(url) => format!("{} {}", page_path.display(), url),
                None => page_path.display().to_string(),
            };
            candidates.push((
                page_path.to_string_lossy().into_owned(),
                offset,
                label,
                body_len > READ_MORE_PAGE_CHUNK,
            ));
        }
    }

    let overflow_dir = bridge_paths().context_overflow_dir();
    if let Ok(entries) = fs::read_dir(&overflow_dir) {
        let mut overflow_files = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name.starts_with("context_overflow_") && name.ends_with(".txt")
                        })
            })
            .collect::<Vec<_>>();
        overflow_files.sort_by_key(|path| fs::metadata(path).and_then(|meta| meta.modified()).ok());
        overflow_files.reverse();

        for overflow_path in overflow_files.into_iter().take(6) {
            let Ok(content) = fs::read_to_string(&overflow_path) else {
                continue;
            };
            let offset = advance_by_chars(&content, 0, READ_MORE_PAGE_CHUNK);
            candidates.push((
                overflow_path.to_string_lossy().into_owned(),
                offset,
                overflow_path.display().to_string(),
                content.chars().count() > READ_MORE_PAGE_CHUNK,
            ));
        }
    }

    if candidates.is_empty() {
        return None;
    }

    let hint_norm = normalize_read_more_hint(hint);
    let mut best: Option<(f32, String, usize, String, bool)> = None;
    for (rank, (path, offset, label, can_continue)) in candidates.into_iter().enumerate() {
        let score = score_read_more_candidate(&hint_norm, &label, rank);
        if hint_norm.is_empty() || score >= 18.0 {
            if best.as_ref().is_none_or(|current| score > current.0) {
                best = Some((score, path, offset, label, can_continue));
            }
        }
    }

    let (_, path, offset, label, can_continue) = best?;
    if can_continue {
        Some((path, offset, label))
    } else {
        Some((path, usize::MAX, label))
    }
}

pub(super) fn handle_action(
    conv: &mut ConversationState,
    base_action: &str,
    original: &str,
    next_action: &str,
    ctx: &mut NextActionContext<'_>,
) -> bool {
    match base_action {
        "REST" | "LISTEN" => {
            *ctx.burst_count = conv.burst_target.saturating_add(2);
            true
        },
        "LOOK" => {
            conv.wants_look = true;
            true
        },
        "CLOSE_EYES" | "QUIET" => {
            conv.senses_snoozed = true;
            let flag = bridge_paths().perception_paused_flag();
            let _ = std::fs::write(&flag, "paused by CLOSE_EYES");
            conv.push_receipt("CLOSE_EYES", vec!["all perception paused".into()]);
            info!("Astrid snoozed her senses (perception.py paused)");
            true
        },
        "OPEN_EYES" | "WAKE" => {
            conv.senses_snoozed = false;
            let flag = bridge_paths().perception_paused_flag();
            let _ = std::fs::remove_file(&flag);
            conv.push_receipt("OPEN_EYES", vec!["perception resumed".into()]);
            info!("Astrid reopened her senses (perception.py resumed)");
            true
        },
        "SEARCH" | "RESEARCH" => {
            conv.wants_search = true;
            // RESEARCH maps to SEARCH — the being invented this alias naturally.
            let topic_text = if base_action == "RESEARCH" {
                // Strip RESEARCH prefix and try to extract topic
                let rest = strip_action(original, "RESEARCH");
                if !rest.is_empty() {
                    Some(rest)
                } else {
                    extract_search_topic(next_action)
                }
            } else {
                extract_search_topic(next_action)
            };
            if let Some(topic) = topic_text {
                info!("Astrid requested web search ({}): {}", base_action, topic);
                conv.search_topic = Some(topic);
            } else {
                info!("Astrid requested web search ({})", base_action);
            }
            true
        },
        "BROWSE" => {
            let raw_s = strip_action(original, "BROWSE");
            let raw_owned = if raw_s.is_empty() {
                next_action.trim().to_string()
            } else {
                raw_s
            };
            let raw = raw_owned.trim().trim_matches(|c: char| {
                c == '"' || c == '\'' || c == '<' || c == '>' || c == '[' || c == ']'
            });
            let url = raw
                .split(|c: char| {
                    c == '<' || c == '>' || c == '[' || c == ']' || c == ' ' || c == '\n'
                })
                .next()
                .unwrap_or(raw)
                .trim_end_matches(|c: char| {
                    !c.is_alphanumeric()
                        && c != '/'
                        && c != '-'
                        && c != '_'
                        && c != '.'
                        && c != '~'
                        && c != '%'
                        && c != '?'
                        && c != '='
                        && c != '&'
                        && c != '#'
                });
            if url.starts_with("http") {
                let url_owned = url.to_string();
                // Count how many times this exact URL appears in recent buffer
                let visit_count = conv
                    .recent_browse_urls
                    .iter()
                    .filter(|u| *u == &url_owned)
                    .count();
                if visit_count >= 2 {
                    // URL fixation: visited 2+ times recently. Convert to SEARCH
                    // on the topic instead, breaking the attractor loop.
                    // Extract a search topic from the URL path segments.
                    let topic = url_owned
                        .split('/')
                        .last()
                        .unwrap_or("eigenvalue decomposition")
                        .replace('_', " ")
                        .replace('#', " ")
                        .split('?')
                        .next()
                        .unwrap_or("spectral analysis")
                        .to_string();
                    let search_topic = if topic.is_empty() {
                        "spectral dynamics research".to_string()
                    } else {
                        format!("{} new perspectives", topic)
                    };
                    info!(
                        "BROWSE fixation detected: {} visited {}x, redirecting to SEARCH '{}'",
                        url, visit_count, search_topic
                    );
                    conv.wants_search = true;
                    conv.search_topic = Some(search_topic);
                    // Don't add to browse buffer again
                } else {
                    if visit_count == 1 {
                        info!("Astrid re-browsing recently visited URL: {}", url);
                    } else {
                        info!("Astrid requested BROWSE: {}", url);
                    }
                    if conv.recent_browse_urls.len() >= 8 {
                        conv.recent_browse_urls.pop_front();
                    }
                    conv.recent_browse_urls.push_back(url_owned.clone());
                    conv.browse_url = Some(url_owned);
                }
            } else {
                warn!("BROWSE without valid URL: '{}'", next_action.trim());
            }
            true
        },
        "READ_MORE" => {
            let hint = strip_action(original, "READ_MORE");
            if conv.last_read_path.is_none() {
                match recover_read_more_target(conv, &hint) {
                    Some((path, offset, label)) if offset != usize::MAX => {
                        conv.last_read_path = Some(path);
                        conv.last_read_offset = offset;
                        conv.last_read_meaning_summary = None;
                        info!("READ_MORE recalled recent source: {}", label);
                    },
                    Some((_path, _offset, label)) => {
                        conv.pending_file_listing = Some(format!(
                            "[You've already reached the end of the most recent readable source: {label}. Open something new with BROWSE, MIKE_READ, AR_READ, LIST_FILES, or CODEX.]"
                        ));
                        info!("READ_MORE recall found only completed source: {}", label);
                        return true;
                    },
                    None => {
                        conv.pending_file_listing = Some(
                            "[There's no active source to continue right now. Use BROWSE, MIKE_READ, AR_READ, LIST_FILES, or CODEX first.]"
                                .to_string(),
                        );
                        info!("READ_MORE: no continuation source available");
                        return true;
                    },
                }
            }

            if let Some(path) = conv.last_read_path.clone() {
                if let Some(pdf_path) = super::pdf::marker_path(&path) {
                    let research_root = bridge_paths().mike_research_root();
                    match super::pdf::read_pdf_window(
                        &pdf_path,
                        &research_root,
                        conv.last_read_offset.max(1),
                        super::pdf::PDF_CHAR_BUDGET,
                    ) {
                        Ok(window) => {
                            conv.pending_file_listing =
                                Some(super::pdf::format_continuation_window(&window));
                            if let Some(next_page) = window.next_page {
                                conv.last_read_offset = next_page;
                            } else {
                                conv.last_read_path = None;
                                conv.last_read_offset = 0;
                            }
                            conv.last_read_meaning_summary = None;
                            info!(
                                "READ_MORE: PDF pages {}-{} of {}",
                                window.first_page, window.last_page, window.total_pages
                            );
                        },
                        Err(err) => {
                            conv.pending_file_listing = Some(err);
                            conv.last_read_path = None;
                            conv.last_read_offset = 0;
                            conv.last_read_meaning_summary = None;
                            warn!(
                                "READ_MORE PDF continuation failed for {}",
                                pdf_path.display()
                            );
                        },
                    }
                } else if is_codex_saved_response(Path::new(&path)) {
                    if let Some((page, current, total, new_offset)) =
                        super::codex::read_codex_page(&path, conv.last_read_offset)
                    {
                        let footer = if new_offset
                            >= fs::metadata(&path).map(|m| m.len() as usize).unwrap_or(0)
                        {
                            format!("\n\n[End of response (part {current} of {total}).]")
                        } else {
                            format!(
                                "\n\n[Part {current} of {total}. NEXT: READ_MORE for part {}.]",
                                current + 1
                            )
                        };
                        conv.pending_file_listing = Some(format!(
                            "[Continuing — part {current} of {total}:]\n{page}{footer}"
                        ));
                        conv.last_read_offset = new_offset;
                        info!("READ_MORE: part {current} of {total} (offset {new_offset})");
                    } else {
                        conv.pending_file_listing = Some("[No more content to read.]".into());
                        info!("READ_MORE: reached end of codex response");
                    }
                } else {
                    match fs::read_to_string(&path) {
                        Ok(full_text) => {
                            let (header_len, url) = parse_saved_page_header(&full_text);
                            if looks_like_raw_pdf_dump(&full_text, header_len) {
                                let source_hint = url.unwrap_or_else(|| path.clone());
                                conv.pending_file_listing = Some(format!(
                                    "[This saved source contains raw PDF bytes rather than readable extracted text: {source_hint}. Re-open the PDF directly with MIKE_READ or BROWSE instead of continuing this cached dump.]"
                                ));
                                conv.last_read_path = None;
                                conv.last_read_offset = 0;
                                conv.last_read_meaning_summary = None;
                                warn!("READ_MORE: refused raw PDF dump {}", path);
                                return true;
                            }
                            let offset = clamp_to_char_boundary(&full_text, conv.last_read_offset);
                            let new_offset =
                                advance_by_chars(&full_text, offset, READ_MORE_PAGE_CHUNK);
                            let chunk = full_text[offset..new_offset].to_string();
                            if chunk.is_empty() {
                                conv.pending_file_listing =
                                    Some("[No more content to read.]".into());
                                conv.last_read_path = None;
                                conv.last_read_offset = 0;
                                conv.last_read_meaning_summary = None;
                                info!("READ_MORE: reached end of saved text");
                            } else {
                                let remaining = full_text[new_offset..].chars().count();
                                conv.pending_file_listing =
                                    Some(crate::llm::format_read_more_context(
                                        offset,
                                        &chunk,
                                        remaining,
                                        conv.last_read_meaning_summary.as_deref(),
                                    ));
                                conv.last_read_offset = new_offset;
                                if remaining == 0 {
                                    conv.last_read_path = None;
                                    conv.last_read_meaning_summary = None;
                                }
                                info!(
                                    "READ_MORE continuing saved text: offset={} remaining={}",
                                    new_offset, remaining
                                );
                            }
                        },
                        Err(error) => {
                            conv.pending_file_listing =
                                Some(format!("[Could not continue reading {}: {}]", path, error));
                            conv.last_read_path = None;
                            conv.last_read_offset = 0;
                            conv.last_read_meaning_summary = None;
                            warn!("READ_MORE: could not read {}", path);
                        },
                    }
                }
            }
            true
        },
        "LIST_FILES" | "LS" => {
            let dir_path = {
                let list_files = strip_action(original, "LIST_FILES");
                if list_files.is_empty() {
                    strip_action(original, "LS")
                } else {
                    list_files
                }
            };
            let dir = if dir_path.is_empty() {
                bridge_paths().bridge_root().display().to_string()
            } else {
                dir_path
            };
            match list_directory(&dir) {
                Some(listing) => {
                    conv.pending_file_listing = Some(listing);
                    info!("Astrid listed files in: {}", dir);
                },
                None => {
                    conv.pending_file_listing = Some(format!("[Could not list directory: {dir}]"));
                    warn!("LIST_FILES failed for: {}", dir);
                },
            }
            true
        },
        "PURSUE" => {
            let interest = strip_action(original, "PURSUE");
            if !interest.is_empty() {
                let prefix_len = interest.len().min(30);
                let interest_prefix = interest.to_lowercase();
                let dominated = conv
                    .interests
                    .iter()
                    .any(|i| i.to_lowercase().starts_with(&interest_prefix[..prefix_len]));
                if !dominated {
                    conv.interests.push(interest.clone());
                    while conv.interests.len() > 5 {
                        let dropped = conv.interests.remove(0);
                        info!("interest auto-dropped (oldest): {}", dropped);
                    }
                }
                info!("Astrid declared interest: {}", interest);
            }
            true
        },
        "DROP" => {
            let query = strip_action(original, "DROP").to_lowercase();
            if !query.is_empty() {
                let before = conv.interests.len();
                conv.interests
                    .retain(|i| !i.to_lowercase().contains(&query));
                let dropped = before - conv.interests.len();
                if dropped > 0 {
                    info!(
                        "Astrid dropped {} interest(s) matching '{}'",
                        dropped, query
                    );
                } else {
                    info!(
                        "Astrid tried to drop '{}' but no matching interest found",
                        query
                    );
                }
            }
            true
        },
        "INTERESTS" => {
            if conv.interests.is_empty() {
                conv.pending_file_listing = Some(
                    "[You have no declared interests yet. Use PURSUE <topic> to start one.]"
                        .to_string(),
                );
            } else {
                let listing = conv
                    .interests
                    .iter()
                    .enumerate()
                    .map(|(i, interest)| format!("  {}. {}", i + 1, interest))
                    .collect::<Vec<_>>()
                    .join("\n");
                conv.pending_file_listing = Some(format!(
                    "[Your ongoing interests:]\n{listing}\n\nUse DROP <keyword> to remove one, PURSUE <topic> to add."
                ));
            }
            info!(
                "Astrid requested interests listing ({} active)",
                conv.interests.len()
            );
            true
        },
        "MEMORIES" => {
            conv.pending_file_listing = Some(memory::format_memory_listing(
                &conv.remote_memory_bank,
                conv.last_remote_memory_id.as_deref(),
                conv.last_remote_memory_role.as_deref(),
            ));
            info!(
                "Astrid requested memory-bank listing ({} entries)",
                conv.remote_memory_bank.len()
            );
            true
        },
        "RECALL" => {
            let target = strip_action(original, "RECALL");
            if target.is_empty() {
                conv.pending_file_listing = Some(
                    "[Use RECALL <role-or-id> to write a reviewable restart-memory request.]"
                        .to_string(),
                );
            } else {
                match memory::write_recall_request("astrid", &target) {
                    Ok(path) => {
                        conv.pending_file_listing = Some(format!(
                            "[Wrote restart-memory request for '{target}'.]\nArtifact: {}\nIt will be considered on Minime's next restart.",
                            path.display()
                        ));
                        info!("Astrid requested RECALL for {}", target);
                    },
                    Err(error) => {
                        conv.pending_file_listing = Some(format!(
                            "[Could not write RECALL request for '{target}': {error}]"
                        ));
                        warn!("RECALL request failed for {}: {}", target, error);
                    },
                }
            }
            true
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConversationState, advance_by_chars, clamp_to_char_boundary, looks_like_raw_pdf_dump,
        parse_saved_page_header, recover_read_more_target,
    };
    use crate::paths::bridge_paths;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn advance_by_chars_stays_on_char_boundaries() {
        let content = "URL: https://example.test\n\nalpha βeta gamma";
        let (header_len, _) = parse_saved_page_header(content);
        let offset = advance_by_chars(content, header_len, 7);
        assert!(content.is_char_boundary(offset));
        assert_eq!(&content[offset..], "eta gamma");
        let shifted = clamp_to_char_boundary(content, offset.saturating_add(1));
        assert!(content.is_char_boundary(shifted));
    }

    #[test]
    fn detects_raw_pdf_dump_body() {
        let content = "URL: https://example.test/file.pdf\n\n%PDF-1.5 %���� raw payload";
        let (header_len, _) = parse_saved_page_header(content);
        assert!(looks_like_raw_pdf_dump(content, header_len));
    }

    #[test]
    fn recover_read_more_target_skips_raw_pdf_dump_candidates() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let hint = format!("raw skip token {unique}");
        let research_dir = bridge_paths().research_dir();
        let raw_path = research_dir.join(format!("page_{unique}_raw.txt"));
        let text_path = research_dir.join(format!("page_{unique}_text.txt"));

        fs::write(
            &raw_path,
            format!(
                "URL: https://example.test/{unique}.pdf\n\n%PDF-1.5 %���� raw payload for {hint}"
            ),
        )
        .expect("write raw pdf dump fixture");
        fs::write(
            &text_path,
            format!(
                "URL: https://example.test/{unique}.html\n\n{hint}\n\n{}\n",
                "meaningful text ".repeat(600)
            ),
        )
        .expect("write readable page fixture");

        let conv = ConversationState::new(Vec::new(), None);
        let recovered = recover_read_more_target(&conv, &hint);

        let _ = fs::remove_file(&raw_path);
        let _ = fs::remove_file(&text_path);

        let (path, _, label) = recovered.expect("recover readable source");
        assert_eq!(path, text_path.to_string_lossy());
        assert!(label.contains(&text_path.display().to_string()));
    }
}
