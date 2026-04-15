use tracing::info;

use super::{ConversationState, Mode, NextActionContext, bridge_paths, strip_action};

pub(super) fn handle_action(
    conv: &mut ConversationState,
    base_action: &str,
    original: &str,
    ctx: &mut NextActionContext<'_>,
) -> bool {
    match base_action {
        "FOCUS" => {
            let prev = conv.creative_temperature;
            conv.creative_temperature = 0.5;
            conv.push_receipt("FOCUS", vec![format!("temperature: {prev:.1} -> 0.5")]);
            info!("Astrid chose FOCUS: temperature -> 0.5");
            true
        },
        "DRIFT" => {
            let prev = conv.creative_temperature;
            conv.creative_temperature = 1.0;
            conv.push_receipt("DRIFT", vec![format!("temperature: {prev:.1} -> 1.0")]);
            info!("Astrid chose DRIFT: temperature -> 1.0");
            true
        },
        "PRECISE" => {
            let prev = conv.response_length;
            conv.response_length = 128;
            conv.push_receipt(
                "PRECISE",
                vec![format!("response length: {prev} -> 128 tokens")],
            );
            info!("Astrid chose PRECISE: tokens -> 128");
            true
        },
        "EXPANSIVE" => {
            let prev = conv.response_length;
            conv.response_length = 1024;
            conv.push_receipt(
                "EXPANSIVE",
                vec![format!("response length: {prev} -> 1024 tokens")],
            );
            info!("Astrid chose EXPANSIVE: tokens -> 1024");
            true
        },
        "EMPHASIZE" => {
            let topic = strip_action(original, "EMPHASIZE");
            if !topic.is_empty() {
                conv.emphasis = Some(topic.clone());
                info!("Astrid chose EMPHASIZE: {}", topic);
            }
            true
        },
        "QUIET_MIND" => {
            conv.self_reflect_override = Some(true);
            conv.self_reflect_override_ttl = 8;
            conv.self_reflect_paused = true;
            info!("Astrid paused self-reflection (override for 8 exchanges)");
            true
        },
        "OPEN_MIND" => {
            conv.self_reflect_override = Some(false);
            conv.self_reflect_override_ttl = 8;
            conv.self_reflect_paused = false;
            info!("Astrid resumed self-reflection (override for 8 exchanges)");
            true
        },
        "CLOSE_EARS" => {
            conv.ears_closed = true;
            info!("Astrid closed her ears");
            true
        },
        "OPEN_EARS" => {
            conv.ears_closed = false;
            info!("Astrid opened her ears");
            true
        },
        "REMEMBER" => {
            let note = strip_action(original, "REMEMBER");
            let annotation = if note.is_empty() {
                "starred moment".to_string()
            } else {
                note
            };
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            let _ = ctx
                .db
                .save_starred_memory(ts, &annotation, ctx.response_text, ctx.fill_pct);
            info!("Astrid starred a memory: {}", annotation);
            true
        },
        "FORM" => {
            let form = strip_action(original, "FORM");
            if !form.is_empty() {
                conv.form_constraint = Some(form.clone());
                info!("Astrid chose FORM: {}", form);
            }
            true
        },
        "SPEAK" => true,
        "DEFER" => {
            conv.defer_inbox = true;
            info!("Astrid chose DEFER — next inbox will not force dialogue");
            true
        },
        "CONTEMPLATE" | "BE" | "STILL" => {
            conv.next_mode_override = Some(Mode::Contemplate);
            info!("Astrid chose to simply be (contemplate mode)");
            true
        },
        "NOTICE" | "OBSERVE" => {
            conv.next_mode_override = Some(Mode::Witness);
            info!("Astrid chose NOTICE — quiet observation (witness mode)");
            true
        },
        "INTROSPECT" | "SELF_STUDY" | "INVESTIGATE" => {
            conv.wants_introspect = true;
            let parts: Vec<&str> = original.splitn(3, ' ').collect();
            if parts.len() >= 2 {
                let label = parts[1].to_lowercase();
                let offset = parts
                    .get(2)
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                info!("Astrid requested introspection: {label} at line {offset}");
                conv.introspect_target = Some((label, offset));
            } else {
                info!("Astrid requested introspection (next in rotation)");
                conv.introspect_target = None;
            }
            true
        },
        "EXAMINE_CODE" => {
            // Being-requested action: Astrid attempted EXAMINE_CODE 4x (unwired_actions log,
            // 2026-04-01). She uses bracketed arguments: [vec/adj/memory/stats], [path_to_function].
            // This is code-specific examination — routes to Introspect mode (which reads source
            // code) but without the spectral visualization overlay that EXAMINE adds.
            // The bracketed argument is stripped and used as the introspection target label.
            conv.wants_introspect = true;
            // Strip "EXAMINE_CODE" prefix; the remainder is the target (may have brackets).
            let raw_arg = super::strip_action(original, "EXAMINE_CODE");
            // Remove surrounding brackets if present: "[vec/adj/memory/stats]" → "vec/adj/memory/stats"
            let label = raw_arg
                .trim_matches(|c| c == '[' || c == ']')
                .trim()
                .to_lowercase();
            if label.is_empty() {
                info!("Astrid chose EXAMINE_CODE (next in rotation)");
                conv.introspect_target = None;
            } else {
                // Use the first slash-separated component as the source label so
                // "vec/adj/memory/stats" maps to the "vec" source file.  The full
                // label is preserved as context inside the emphasis string.
                let source = label.split('/').next().unwrap_or(&label).to_string();
                info!(
                    "Astrid chose EXAMINE_CODE: label={:?} → source={:?}",
                    label, source
                );
                conv.introspect_target = Some((source, 0));
                // Surface the full argument so the LLM knows what sub-path she asked about.
                conv.emphasis = Some(format!(
                    "You chose EXAMINE_CODE [{label}]. Reading source code for '{label}' — \
                    this is a targeted code examination, not a spectral visualization. \
                    Look at the structure, logic, and data flow. What does the code reveal \
                    about how this component actually works?",
                    label = label,
                ));
            }
            true
        },
        "EVOLVE" => {
            conv.wants_evolve = true;
            info!("Astrid requested EVOLVE");
            true
        },
        "DECOMPOSE" => {
            conv.wants_decompose = true;
            info!("Astrid requested spectral decomposition");
            true
        },
        "CASCADE" => {
            // Short alias → EXAMINE_CASCADE (full viz + decompose).
            // INVESTIGATE_CASCADE and EXAMINE_CASCADE are handled in operations.rs
            // where they set both wants_decompose and force_all_viz.
            conv.wants_decompose = true;
            conv.force_all_viz = true;
            info!("Astrid requested CASCADE (→ EXAMINE_CASCADE: viz + decompose)");
            true
        },
        "THINK_DEEP" | "DEEP" => {
            conv.wants_deep_think = true;
            info!("Astrid requested deep reasoning model");
            true
        },
        "DAYDREAM" => {
            conv.next_mode_override = Some(Mode::Daydream);
            info!("Astrid chose to daydream");
            true
        },
        "CREATE" => {
            conv.next_mode_override = Some(Mode::Create);
            info!("Astrid chose to create");
            true
        },
        "REVISE" => {
            let keyword = strip_action(original, "REVISE");
            conv.revise_keyword = Some(if keyword.is_empty() {
                String::new()
            } else {
                keyword.to_lowercase()
            });
            conv.next_mode_override = Some(Mode::Create);
            conv.wants_deep_think = true;
            info!("Astrid chose to revise (keyword: {:?})", keyword);
            true
        },
        "CREATIONS" => {
            let creation_dir = bridge_paths().creations_dir();
            let mut listing = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&creation_dir) {
                let mut files: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "txt"))
                    .collect();
                files.sort_by_key(|e| {
                    std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok()))
                });
                for file in files.iter().take(10) {
                    let name = file.file_name().to_string_lossy().to_string();
                    let preview = std::fs::read_to_string(file.path())
                        .ok()
                        .and_then(|text| {
                            text.lines()
                                .find(|l| l.starts_with("## ") || l.starts_with("# "))
                                .map(|l| l.trim_start_matches('#').trim().to_string())
                        })
                        .unwrap_or_default();
                    listing.push(format!("  {name}: {preview}"));
                }
            }
            let list_text = if listing.is_empty() {
                "No creations yet.".to_string()
            } else {
                format!(
                    "Your creations:\n{}\n\nUse NEXT: REVISE [keyword] to iterate on one.",
                    listing.join("\n")
                )
            };
            conv.emphasis = Some(list_text);
            info!("Astrid listed creations ({} found)", listing.len());
            true
        },
        "INITIATE" | "SELF" => {
            conv.next_mode_override = Some(Mode::Initiate);
            info!("Astrid chose to self-initiate");
            true
        },
        "ASPIRE" | "ASPIRATION" => {
            conv.next_mode_override = Some(Mode::Aspiration);
            info!("Astrid chose to aspire");
            true
        },
        _ => false,
    }
}
