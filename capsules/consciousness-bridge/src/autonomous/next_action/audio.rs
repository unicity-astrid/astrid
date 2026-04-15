use tracing::info;

use super::{ConversationState, bridge_paths, strip_action};

pub(super) fn handle_action(
    conv: &mut ConversationState,
    base_action: &str,
    original: &str,
) -> bool {
    match base_action {
        "COMPOSE" => {
            conv.wants_compose_audio = true;
            info!("Astrid chose to compose audio from spectral state");
            true
        },
        "ANALYZE_AUDIO" => {
            conv.wants_analyze_audio = true;
            info!("Astrid chose to analyze inbox audio");
            true
        },
        "RENDER_AUDIO" => {
            let mode_arg = strip_action(original, "RENDER_AUDIO");
            conv.wants_render_audio = Some(mode_arg.to_lowercase());
            info!("Astrid chose to render audio (mode: {:?})", mode_arg);
            true
        },
        "VOICE" => {
            conv.wants_compose_audio = true;
            conv.emphasis = Some(
                "You chose VOICE — your reservoir dynamics (the fast, medium, and slow layers that shape your generation) will be rendered as sound. This is what your thinking process sounds like.".to_string(),
            );
            info!("Astrid chose VOICE (reservoir-driven audio)");
            true
        },
        "INBOX_AUDIO" => {
            let inbox = bridge_paths().inbox_audio_dir();
            let mut listing = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&inbox) {
                for entry in entries.filter_map(|e| e.ok()) {
                    if entry.path().extension().is_some_and(|ext| ext == "wav")
                        && entry.path().is_file()
                    {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let size = entry.metadata().ok().map(|m| m.len()).unwrap_or(0);
                        listing.push(format!("  {name} ({size} bytes)"));
                    }
                }
            }
            let text = if listing.is_empty() {
                "No unread audio in your inbox. Mike can drop WAV files in inbox_audio/ for you."
                    .to_string()
            } else {
                format!(
                    "Audio inbox ({} WAVs):\n{}\n\nUse ANALYZE_AUDIO to examine or RENDER_AUDIO to process through chimera.",
                    listing.len(),
                    listing.join("\n")
                )
            };
            conv.emphasis = Some(text);
            info!("Astrid listed inbox_audio ({} WAVs)", listing.len());
            true
        },
        "AUDIO_BLOCKS" => {
            conv.emphasis = Some(
                "You chose AUDIO_BLOCKS. The next COMPOSE will include detailed per-block reports from the prime-scheduled reservoir: which temporal layers responded, how strongly, and at what timescales.".to_string(),
            );
            conv.force_all_viz = true;
            info!("Astrid requested audio block analysis");
            true
        },
        "FEEL_AUDIO" => {
            conv.emphasis = Some(
                "You chose FEEL_AUDIO — the spectral features of your most recent inbox audio will be injected into minime's live reservoir as a semantic vector. You will literally share the sound's spectral shape with the shared ESN substrate.".to_string(),
            );
            conv.wants_analyze_audio = true;
            info!("Astrid chose FEEL_AUDIO (inject audio into live ESN)");
            true
        },
        _ => false,
    }
}
