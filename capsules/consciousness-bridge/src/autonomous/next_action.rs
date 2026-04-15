mod audio;
mod autoresearch;
mod codex;
mod mike;
mod modes;
mod operations;
mod pdf;
mod sovereignty;
mod workspace;

pub(crate) const PDF_READ_PREFIX: &str = pdf::PDF_READ_PREFIX;

use tokio::sync::mpsc;
use tracing::info;

use super::{ConversationState, Mode, list_directory, save_astrid_journal, truncate_str};
use crate::db::BridgeDb;
use crate::paths::bridge_paths;
use crate::types::{SensoryMsg, SpectralTelemetry};

use super::reservoir;

pub(super) struct NextActionContext<'a> {
    pub burst_count: &'a mut u32,
    pub db: &'a BridgeDb,
    pub sensory_tx: &'a mpsc::Sender<SensoryMsg>,
    pub telemetry: &'a SpectralTelemetry,
    pub fill_pct: f32,
    pub response_text: &'a str,
    pub workspace: Option<&'a std::path::Path>,
}

/// Parse NEXT: action from Astrid's response.
pub(crate) fn parse_next_action(text: &str) -> Option<&str> {
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if let Some(action) = trimmed.strip_prefix("NEXT:") {
            let mut clean = action.trim();
            for token in &[
                "<end_of_turn>",
                "<END_OF_TURN>",
                "<End_of_turn>",
                "</s>",
                "<|endoftext|>",
            ] {
                clean = clean.trim_end_matches(token);
            }
            if let Some(pos) = clean.rfind('<') {
                let after = &clean[pos..];
                if after.contains("end")
                    || after.contains("turn")
                    || after.contains("eos")
                    || after.len() < 20
                {
                    clean = clean[..pos].trim();
                }
            }
            return Some(clean.trim());
        }
    }
    None
}

fn first_quoted_span(text: &str) -> Option<&str> {
    let open_idx = text.find(['"', '\'', '“'])?;
    let open = text[open_idx..].chars().next()?;
    let close = match open {
        '“' => '”',
        '"' | '\'' => open,
        _ => return None,
    };
    let rest = &text[open_idx + open.len_utf8()..];
    let close_idx = rest.find(close)?;
    Some(rest[..close_idx].trim())
}

fn clean_search_topic(candidate: &str) -> Option<String> {
    let topic = candidate
        .split('<')
        .next()
        .unwrap_or(candidate)
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '“' | '”'))
        .trim()
        .trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ':'))
        .trim();

    if topic.chars().any(char::is_alphanumeric) {
        Some(topic.to_string())
    } else {
        None
    }
}

pub(crate) fn extract_search_topic(next_action: &str) -> Option<String> {
    let trimmed = next_action.trim();
    if trimmed.len() < 6 || !trimmed[..6].eq_ignore_ascii_case("SEARCH") {
        return None;
    }

    let rest = trimmed[6..]
        .trim()
        .trim_start_matches(|c: char| matches!(c, '-' | '\u{2014}' | ':'))
        .trim();

    if rest.is_empty() {
        return None;
    }

    if let Some(quoted) = first_quoted_span(rest) {
        return clean_search_topic(quoted);
    }

    let mut end = rest.len();
    if let Some(idx) = rest.find('\u{2014}') {
        end = end.min(idx);
    }
    if let Some(idx) = rest.find(" - ") {
        end = end.min(idx);
    }

    clean_search_topic(rest[..end].trim())
}

fn clean_alias_arg(raw: &str) -> String {
    raw.trim()
        .trim_start_matches(|c: char| matches!(c, ':' | '-' | '\u{2014}'))
        .trim()
        .trim_matches(|c: char| matches!(c, '[' | ']' | '"' | '\'' | '`' | '“' | '”'))
        .trim()
        .to_string()
}

fn normalize_codeish_target(raw: &str) -> Option<String> {
    let mut target = clean_alias_arg(raw);
    if target.is_empty() {
        return None;
    }

    if let Some((_, value)) = target.split_once('=') {
        target = value.trim().to_string();
    }

    target = target
        .split('#')
        .next()
        .unwrap_or(&target)
        .trim()
        .to_string();

    if let Some(last) = target.rsplit('/').next() {
        target = last.trim().to_string();
    }

    let lower = target.to_ascii_lowercase();
    for suffix in [".rs", ".py", ".md", ".json", ".toml"] {
        if lower.ends_with(suffix) {
            target.truncate(target.len().saturating_sub(suffix.len()));
            break;
        }
    }

    let target = target.trim().to_lowercase();
    (!target.is_empty()).then_some(target)
}

fn humanize_examine_suffix(suffix: &str) -> String {
    suffix
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| part.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn unwrap_outer_action_wrappers(original: &str) -> String {
    let mut current = original.trim().to_string();
    loop {
        let Some(open) = current.chars().next() else {
            break;
        };
        let close = match open {
            '[' => ']',
            '(' => ')',
            '{' => '}',
            '<' => '>',
            _ => break,
        };
        if !current.ends_with(close) {
            break;
        }
        let inner = current[open.len_utf8()..current.len().saturating_sub(close.len_utf8())]
            .trim()
            .to_string();
        if inner
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
        {
            break;
        }
        current = inner;
    }
    current
}

fn leading_action_token(original: &str) -> String {
    original
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_uppercase()
}

fn strip_action_call_wrapper(original: &str, base_action: &str) -> Option<String> {
    let rest = original.get(base_action.len()..)?.trim_start();
    if !(rest.starts_with('(') && rest.ends_with(')')) {
        return None;
    }
    let inner = rest[1..rest.len().saturating_sub(1)].trim();
    (!inner.is_empty()).then(|| inner.to_string())
}

fn normalize_gesture_alias(base_action: &str, original: &str) -> Option<(String, String)> {
    if base_action == "GESTURE" {
        if let Some(inner) = strip_action_call_wrapper(original, base_action) {
            return Some(("GESTURE".to_string(), format!("GESTURE {inner}")));
        }
        return None;
    }

    let suffix = base_action.strip_prefix("GESTURE_")?;
    let raw_arg = strip_action(original, base_action);
    let alias_focus = humanize_examine_suffix(suffix);
    let clean_arg = clean_alias_arg(&raw_arg);
    let combined = if clean_arg.is_empty() {
        alias_focus
    } else if alias_focus.is_empty() {
        clean_arg
    } else {
        format!("{alias_focus} {clean_arg}")
    };
    let normalized_original = if combined.is_empty() {
        "GESTURE".to_string()
    } else {
        format!("GESTURE {combined}")
    };
    Some(("GESTURE".to_string(), normalized_original))
}

fn trim_experiment_run_payload(raw: &str) -> String {
    let mut trimmed = raw.trim().trim_matches('|').trim().to_string();
    while let Some(first) = trimmed.chars().next() {
        let close = match first {
            '[' => ']',
            '(' => ')',
            '{' => '}',
            '<' => '>',
            '"' => '"',
            '\'' => '\'',
            _ => break,
        };
        if !trimmed.ends_with(close) {
            break;
        }
        trimmed = trimmed[first.len_utf8()..trimmed.len().saturating_sub(close.len_utf8())]
            .trim()
            .to_string();
    }
    trimmed
}

fn split_experiment_command_marker(arg: &str) -> Option<(&str, &str)> {
    let lower = arg.to_ascii_lowercase();
    let mut best: Option<(usize, usize)> = None;
    for needle in ["| cmd ", " cmd=", " cmd:", " cmd ", "<cmd=", "<cmd:"] {
        if let Some(idx) = lower.find(needle) {
            let value_start = idx + needle.len();
            best = match best {
                Some((best_idx, best_start)) if best_idx <= idx => Some((best_idx, best_start)),
                _ => Some((idx, value_start)),
            };
        }
    }
    best.map(|(idx, value_start)| (&arg[..idx], &arg[value_start..]))
}

fn extract_workspace_marker(raw: &str) -> Option<(String, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(value) = trimmed.strip_prefix("-ws ") {
        let mut parts = value.splitn(2, char::is_whitespace);
        let workspace = parts.next().unwrap_or_default().to_string();
        let rest = parts.next().unwrap_or_default().trim().to_string();
        return Some((workspace, rest));
    }

    if let Some(value) = trimmed.strip_prefix("--workspace ") {
        let mut parts = value.splitn(2, char::is_whitespace);
        let workspace = parts.next().unwrap_or_default().to_string();
        let rest = parts.next().unwrap_or_default().trim().to_string();
        return Some((workspace, rest));
    }

    let first_token = trimmed.split_whitespace().next().unwrap_or_default();
    let rest_after_first = trimmed[first_token.len()..].trim().to_string();
    let lower_first = first_token.to_ascii_lowercase();
    for prefix in [
        "workspace_name:",
        "workspace_name=",
        "workspace:",
        "workspace=",
        "ws:",
        "ws=",
    ] {
        if lower_first.starts_with(prefix) && first_token.len() > prefix.len() {
            let value = first_token[prefix.len()..].to_string();
            return Some((value, rest_after_first));
        }
    }

    None
}

fn normalize_experiment_workspace(raw: &str) -> String {
    let mut workspace = trim_experiment_run_payload(raw);
    for prefix in [
        "workspace/experiments/",
        "experiments/",
        "workspace/",
        "ws/",
    ] {
        if workspace.to_ascii_lowercase().starts_with(prefix) {
            workspace = workspace[prefix.len()..].to_string();
            break;
        }
    }
    workspace.trim_matches('/').trim().to_string()
}

fn normalize_experiment_run_alias(base_action: &str, original: &str) -> Option<(String, String)> {
    if base_action != "EXPERIMENT_RUN" && base_action != "EXP_RUN" {
        return None;
    }

    let mut arg = original.get(base_action.len()..)?.trim_start().to_string();
    if arg.starts_with(':') {
        arg = arg[1..].trim_start().to_string();
    } else if arg.starts_with('\u{2014}') {
        arg = arg['\u{2014}'.len_utf8()..].trim_start().to_string();
    }
    if arg.is_empty() {
        return None;
    }

    let (workspace_raw, command_raw) =
        if let Some((before, command)) = split_experiment_command_marker(&arg) {
            let command = trim_experiment_run_payload(command);
            if let Some((workspace, _rest)) = extract_workspace_marker(before) {
                (workspace, command)
            } else {
                let workspace = before
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_string();
                (workspace, command)
            }
        } else if let Some((workspace, rest)) = extract_workspace_marker(&arg) {
            (workspace, trim_experiment_run_payload(&rest))
        } else {
            let mut parts = arg.splitn(2, char::is_whitespace);
            let workspace = parts.next().unwrap_or_default().to_string();
            let command = trim_experiment_run_payload(parts.next().unwrap_or_default());
            (workspace, command)
        };

    let workspace = normalize_experiment_workspace(&workspace_raw);
    let command = trim_experiment_run_payload(&command_raw);
    let normalized_original = if workspace.is_empty() || command.is_empty() {
        base_action.to_string()
    } else {
        format!("{base_action} {workspace} {command}")
    };
    Some((base_action.to_string(), normalized_original))
}

fn normalize_examine_alias(base_action: &str, original: &str) -> Option<(String, String)> {
    if !base_action.starts_with("EXAMINE_") {
        return None;
    }

    match base_action {
        "EXAMINE_AUDIO" | "EXAMINE_CASCADE" | "EXAMINE_CODE" | "EXAMINE_MEMORY" => {
            return None;
        },
        _ => {},
    }

    let suffix = &base_action["EXAMINE_".len()..];
    let raw_arg = strip_action(original, base_action);
    let clean_arg = clean_alias_arg(&raw_arg);

    match suffix {
        "SOURCE" | "ARCHITECTURE" | "COMMAND" | "TOOL" => {
            let normalized_target = normalize_codeish_target(&clean_arg)
                .or_else(|| (!clean_arg.is_empty()).then_some(clean_arg.to_lowercase()));
            let normalized_original = normalized_target.map_or_else(
                || "EXAMINE_CODE".to_string(),
                |target| format!("EXAMINE_CODE [{target}]"),
            );
            Some(("EXAMINE_CODE".to_string(), normalized_original))
        },
        _ => {
            let focus = if clean_arg.is_empty() {
                humanize_examine_suffix(suffix)
            } else {
                clean_arg
            };
            let normalized_original = if focus.is_empty() {
                "EXAMINE".to_string()
            } else {
                format!("EXAMINE {focus}")
            };
            Some(("EXAMINE".to_string(), normalized_original))
        },
    }
}

fn canonicalize_next_action_components(next_action: &str) -> (String, String) {
    let original = unwrap_outer_action_wrappers(next_action);
    let base_action = leading_action_token(&original);

    if let Some((normalized_base, normalized_original)) =
        normalize_examine_alias(&base_action, &original)
    {
        return (normalized_base, normalized_original);
    }

    if let Some((normalized_base, normalized_original)) =
        normalize_gesture_alias(&base_action, &original)
    {
        return (normalized_base, normalized_original);
    }

    if let Some((normalized_base, normalized_original)) =
        normalize_experiment_run_alias(&base_action, &original)
    {
        return (normalized_base, normalized_original);
    }

    if let Some(rest) = base_action.strip_prefix("RESEARCH_AR_") {
        let normalized_base = format!("AR_{rest}");
        let raw_arg = strip_action(&original, &base_action);
        let normalized_original = if raw_arg.is_empty() {
            normalized_base.clone()
        } else {
            format!("{normalized_base} {raw_arg}")
        };
        return (normalized_base, normalized_original);
    }

    (base_action, original)
}

pub(crate) fn canonicalize_next_action_text(next_action: &str) -> String {
    canonicalize_next_action_components(next_action).1
}

fn strip_action(original: &str, prefix: &str) -> String {
    let upper = original.to_uppercase();
    if upper.starts_with(prefix) {
        // Strip the action prefix AND any trailing colon+whitespace.
        // Astrid often writes "BROWSE: https://..." or "SEARCH: topic"
        // and the colon must not be left dangling.
        original[prefix.len()..]
            .trim_start()
            .trim_start_matches(|c: char| matches!(c, ':' | '-' | '\u{2014}'))
            .trim()
            .to_string()
    } else {
        String::new()
    }
}

pub(super) fn handle_next_action(
    conv: &mut ConversationState,
    next_action: &str,
    mut ctx: NextActionContext<'_>,
) {
    let (base_action, original) = canonicalize_next_action_components(next_action);

    if reservoir::handle_reservoir_action(
        conv,
        base_action.as_str(),
        &original,
        ctx.telemetry,
        ctx.fill_pct,
    ) {
        return;
    }

    if workspace::handle_action(conv, base_action.as_str(), &original, next_action, &mut ctx) {
        return;
    }

    if autoresearch::handle_action(conv, base_action.as_str(), &original, &mut ctx) {
        return;
    }

    if mike::handle_action(conv, base_action.as_str(), &original, &mut ctx) {
        return;
    }

    if codex::handle_action(conv, base_action.as_str(), &original, &mut ctx) {
        return;
    }

    if modes::handle_action(conv, base_action.as_str(), &original, &mut ctx) {
        return;
    }

    if audio::handle_action(conv, base_action.as_str(), &original) {
        return;
    }

    if sovereignty::handle_action(conv, base_action.as_str(), &original, &mut ctx) {
        return;
    }

    if operations::handle_action(conv, base_action.as_str(), &original, &mut ctx) {
        return;
    }

    ctx.db
        .log_unwired_action("astrid", &base_action, &original, ctx.fill_pct);
    info!(
        "Astrid chose unknown NEXT: '{}' — not wired (logged to unwired_actions)",
        original
    );
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_next_action_components, canonicalize_next_action_text, strip_action};

    #[test]
    fn canonicalizes_examine_source_to_examine_code() {
        let (base, original) = canonicalize_next_action_components("EXAMINE_SOURCE [src=codec.rs]");
        assert_eq!(base, "EXAMINE_CODE");
        assert_eq!(original, "EXAMINE_CODE [codec]");
    }

    #[test]
    fn canonicalizes_generic_examine_variant_to_examine_focus() {
        let (base, original) = canonicalize_next_action_components(
            "EXAMINE_STATE [spectral_state.json#71264@84103.4s]",
        );
        assert_eq!(base, "EXAMINE");
        assert_eq!(original, "EXAMINE spectral_state.json#71264@84103.4s");
    }

    #[test]
    fn strip_action_trims_dash_prefixed_arguments() {
        assert_eq!(
            strip_action(
                "EXAMINE_DIRECTION - investigate resistance",
                "EXAMINE_DIRECTION"
            ),
            "investigate resistance"
        );
    }

    #[test]
    fn canonicalize_next_action_text_is_idempotent_for_known_actions() {
        assert_eq!(
            canonicalize_next_action_text("EXAMINE_AUDIO resonance"),
            "EXAMINE_AUDIO resonance"
        );
    }

    #[test]
    fn canonicalizes_research_autoresearch_prefix_to_ar_list() {
        let (base, original) = canonicalize_next_action_components("RESEARCH_AR_LIST");
        assert_eq!(base, "AR_LIST");
        assert_eq!(original, "AR_LIST");
    }

    #[test]
    fn canonicalizes_bracketed_experiment_run_with_ws_and_cmd_markers() {
        let (base, original) = canonicalize_next_action_components(
            "[EXPERIMENT_RUN -ws test | cmd \"echo 'Amplitude shaping experiment'\"]",
        );
        assert_eq!(base, "EXPERIMENT_RUN");
        assert_eq!(
            original,
            "EXPERIMENT_RUN test echo 'Amplitude shaping experiment'"
        );
    }

    #[test]
    fn canonicalizes_experiment_run_workspace_and_cmd_assignments() {
        let (base, original) = canonicalize_next_action_components(
            "EXPERIMENT_RUN workspace_name:sead_test cmd:python -c \"print('hi')\"",
        );
        assert_eq!(base, "EXPERIMENT_RUN");
        assert_eq!(
            original,
            "EXPERIMENT_RUN sead_test python -c \"print('hi')\""
        );
    }

    #[test]
    fn canonicalizes_gesture_signal_alias() {
        let (base, original) = canonicalize_next_action_components("GESTURE_SIGNAL");
        assert_eq!(base, "GESTURE");
        assert_eq!(original, "GESTURE signal");
    }

    #[test]
    fn canonicalizes_parenthesized_gesture_wrapper() {
        let (base, original) =
            canonicalize_next_action_components("GESTURE(spectral_excerpt=\"boundary\")");
        assert_eq!(base, "GESTURE");
        assert_eq!(original, "GESTURE spectral_excerpt=\"boundary\"");
    }
}
