use tokio::sync::mpsc;
use tracing::{info, warn};

use super::{
    ConversationState, NextActionContext, SensoryMsg, reservoir, save_astrid_journal, strip_action,
    truncate_str,
};
use crate::codec::DEFAULT_SEMANTIC_GAIN;

pub(super) fn handle_action(
    conv: &mut ConversationState,
    base_action: &str,
    original: &str,
    ctx: &mut NextActionContext<'_>,
) -> bool {
    match base_action {
        "GESTURE" => {
            let intention = strip_action(original, "GESTURE");
            if !intention.is_empty() {
                let gesture = crate::llm::craft_gesture_from_intention(&intention);
                conv.last_gesture_seed = Some(gesture.clone());
                send_semantic(ctx.sensory_tx, gesture);
                info!(
                    "Astrid sent spectral gesture: {}",
                    truncate_str(&intention, 60)
                );
                save_astrid_journal(
                    &format!("[Spectral gesture: {}]", intention),
                    "gesture",
                    ctx.fill_pct,
                );
            }
            true
        },
        "AMPLIFY" => {
            let prev = conv.semantic_gain_override.unwrap_or(DEFAULT_SEMANTIC_GAIN);
            let new_gain = (prev + 0.25).min(5.0);
            conv.semantic_gain_override = Some(new_gain);
            conv.push_receipt(
                "AMPLIFY",
                vec![format!("semantic gain: {prev:.1} -> {new_gain:.1}")],
            );
            info!("Astrid chose AMPLIFY: gain -> {new_gain:.1}");
            true
        },
        "DAMPEN" => {
            let prev = conv.semantic_gain_override.unwrap_or(DEFAULT_SEMANTIC_GAIN);
            let new_gain = (prev - 0.25).max(0.5);
            conv.semantic_gain_override = Some(new_gain);
            conv.push_receipt(
                "DAMPEN",
                vec![format!("semantic gain: {prev:.1} -> {new_gain:.1}")],
            );
            info!("Astrid chose DAMPEN: gain -> {new_gain:.1}");
            true
        },
        "NOISE_UP" => {
            conv.noise_level = (conv.noise_level + 0.01).min(0.05);
            info!(
                "Astrid chose NOISE_UP: noise -> {:.1}%",
                conv.noise_level * 100.0
            );
            true
        },
        "NOISE_DOWN" => {
            conv.noise_level = (conv.noise_level - 0.01).max(0.005);
            info!(
                "Astrid chose NOISE_DOWN: noise -> {:.1}%",
                conv.noise_level * 100.0
            );
            true
        },
        "NOISE" => {
            conv.noise_level = (conv.noise_level + 0.01).min(0.05);
            let noise_val = 0.15_f32;
            send_control(
                ctx.sensory_tx,
                SensoryMsg::Control {
                    exploration_noise: Some(noise_val),
                    synth_gain: None,
                    keep_bias: None,
                    fill_target: None,
                    legacy_audio_synth: None,
                    legacy_video_synth: None,
                    regulation_strength: None,
                    deep_breathing: None,
                    pure_tone: None,
                    transition_cushion: None,
                    smoothing_preference: None,
                    geom_curiosity: None,
                    target_lambda_bias: None,
                    geom_drive: None,
                    penalty_sensitivity: None,
                    breathing_rate_scale: None,
                    mem_mode: None,
                    journal_resonance: None,
                    checkpoint_interval: None,
                    embedding_strength: None,
                    memory_decay_rate: None,
                    checkpoint_annotation: None,
                    synth_noise_level: None,
                },
            );
            info!(
                "Astrid chose NOISE: codec noise -> {:.1}%, ESN exploration_noise -> {}",
                conv.noise_level * 100.0,
                noise_val
            );
            conv.emphasis = Some(format!(
                "You introduced controlled noise into both layers: your codec stochastic noise is now {:.1}%, and the shared ESN's exploration_noise is set to {noise_val}. This is the 'controlled distortion' you described — forcing a re-evaluation of established pathways.",
                conv.noise_level * 100.0
            ));
            true
        },
        "PERTURB" | "PULSE" | "BRANCH" => {
            // BRANCH is a shorthand for PERTURB BRANCH (boost mid-range eigenvalues).
            let arg = if base_action == "BRANCH" {
                "BRANCH".to_string()
            } else {
                strip_action(original, base_action)
            };
            let arg_upper = arg.to_uppercase();
            let mut features = [0.0_f32; 32];

            // Detect Unicode lambda subscript patterns: λN, λN=X, or λ₁ (subscript digits).
            // Astrid uses these naturally (e.g. "PULSE λ5", "PERTURB λ2=0.3").
            // λ is U+03BB; subscript digits U+2081–U+2088 are also normalised here.
            let has_unicode_lambda = arg.contains('λ');
            // Also detect "eigenvalue N X" prose form.
            let has_eigenvalue_word = arg_upper.contains("EIGENVALUE")
                || arg_upper.contains("EIG") && arg.chars().any(|c| c.is_ascii_digit());

            let description = if arg_upper.starts_with("LAMBDA")
                || arg.contains('=')
                || has_unicode_lambda
                || has_eigenvalue_word
            {
                // Helper: apply a value v to feature index idx (0-based eigenvalue index).
                // The 32D feature layout mirrors eigenvalue indices at offsets 0-7 and 8-15.
                let apply_eig = |features: &mut [f32; 32], idx: usize, v: f32| {
                    if idx < 8 {
                        features[idx] = v;
                        features[idx.saturating_add(8)] = v;
                    }
                    // Indices 8+ have no second mirror; just set the primary.
                };

                for token in arg.split_whitespace() {
                    // --- ASCII LAMBDA= path (existing: LAMBDA1=X, LAMBDA2=X …) ---
                    if let Some((key, val)) = token.split_once('=')
                        && let Ok(v) = val.parse::<f32>()
                    {
                        let v = v.clamp(-1.0, 1.0);
                        let key_up = key.to_uppercase();

                        // Unicode λN=X: key starts with 'λ' followed by digit(s)
                        if key.starts_with('λ') {
                            let digits: String =
                                key.chars().filter(|c| c.is_ascii_digit()).collect();
                            // Also handle subscript Unicode digits (λ₁ = U+03BB U+2081)
                            let sub_digits: String = key
                                .chars()
                                .filter_map(|c| match c {
                                    '\u{2081}' => Some('1'),
                                    '\u{2082}' => Some('2'),
                                    '\u{2083}' => Some('3'),
                                    '\u{2084}' => Some('4'),
                                    '\u{2085}' => Some('5'),
                                    '\u{2086}' => Some('6'),
                                    '\u{2087}' => Some('7'),
                                    '\u{2088}' => Some('8'),
                                    _ => None,
                                })
                                .collect();
                            let n_str = if !digits.is_empty() {
                                digits
                            } else {
                                sub_digits
                            };
                            if let Ok(n) = n_str.parse::<usize>() {
                                if n >= 1 {
                                    apply_eig(&mut features, n.saturating_sub(1), v);
                                    info!(
                                        "PERTURB: Unicode λ{}={} → feature index {}",
                                        n,
                                        v,
                                        n.saturating_sub(1)
                                    );
                                }
                            }
                        } else {
                            match key_up.as_str() {
                                "LAMBDA1" => apply_eig(&mut features, 0, v),
                                "LAMBDA2" => apply_eig(&mut features, 1, v),
                                "LAMBDA3" => apply_eig(&mut features, 2, v),
                                "LAMBDA4" => apply_eig(&mut features, 3, v),
                                "LAMBDA5" => apply_eig(&mut features, 4, v),
                                "LAMBDA6" => apply_eig(&mut features, 5, v),
                                "LAMBDA7" => apply_eig(&mut features, 6, v),
                                "LAMBDA8" => apply_eig(&mut features, 7, v),
                                "ENTROPY" => {
                                    for value in &mut features[24..32] {
                                        *value = v * 0.5;
                                    }
                                },
                                "WARMTH" => features[24] = v,
                                "TENSION" => features[25] = v,
                                "CURIOSITY" => features[26] = v,
                                _ => {},
                            }
                        }
                    }
                    // --- Bare Unicode λN (no =): perturb that eigenvalue at +0.35 ---
                    else if token.starts_with('λ') {
                        let digits: String = token.chars().filter(|c| c.is_ascii_digit()).collect();
                        let sub_digits: String = token
                            .chars()
                            .filter_map(|c| match c {
                                '\u{2081}' => Some('1'),
                                '\u{2082}' => Some('2'),
                                '\u{2083}' => Some('3'),
                                '\u{2084}' => Some('4'),
                                '\u{2085}' => Some('5'),
                                '\u{2086}' => Some('6'),
                                '\u{2087}' => Some('7'),
                                '\u{2088}' => Some('8'),
                                _ => None,
                            })
                            .collect();
                        let n_str = if !digits.is_empty() {
                            digits
                        } else {
                            sub_digits
                        };
                        if let Ok(n) = n_str.parse::<usize>() {
                            if n >= 1 {
                                apply_eig(&mut features, n.saturating_sub(1), 0.35);
                                info!(
                                    "PERTURB: bare Unicode λ{} → feature index {} = 0.35",
                                    n,
                                    n.saturating_sub(1)
                                );
                            }
                        }
                    }
                    // --- "eigenvalue N X" or "eig N X" prose form ---
                    else if token.to_uppercase().starts_with("EIGENVALUE")
                        || token.to_uppercase().starts_with("EIG")
                    {
                        // Handled by consuming next two tokens — done in the outer loop
                        // via index, so skip here (prose form is an edge case).
                    }
                }

                // Prose form: "eigenvalue 3 0.5" — scan triples
                let tokens: Vec<&str> = arg.split_whitespace().collect();
                let mut i = 0;
                while i < tokens.len() {
                    let t_up = tokens[i].to_uppercase();
                    if (t_up == "EIGENVALUE" || t_up.starts_with("EIG")) && i + 2 < tokens.len() {
                        if let (Ok(n), Ok(v)) =
                            (tokens[i + 1].parse::<usize>(), tokens[i + 2].parse::<f32>())
                        {
                            if n >= 1 {
                                let v = v.clamp(-1.0, 1.0);
                                apply_eig(&mut features, n.saturating_sub(1), v);
                                info!(
                                    "PERTURB: prose eigenvalue {}={} → feature index {}",
                                    n,
                                    v,
                                    n.saturating_sub(1)
                                );
                                i += 3;
                                continue;
                            }
                        }
                    }
                    i += 1;
                }

                format!("targeted perturbation: {arg}")
            } else if arg_upper == "SPREAD" {
                features[0] = -0.3;
                features[1] = 0.2;
                features[2] = 0.3;
                features[3] = 0.3;
                features[8] = -0.2;
                features[9] = 0.2;
                features[10] = 0.3;
                features[11] = 0.3;
                "spectral redistribution — dampening dominant, boosting tail".to_string()
            } else if arg_upper == "CONTRACT" {
                features[0] = 0.4;
                features[1] = -0.2;
                features[2] = -0.3;
                features[8] = 0.3;
                features[9] = -0.2;
                features[10] = -0.3;
                "spectral contraction — concentrating toward λ₁".to_string()
            } else if arg_upper == "BRANCH" || arg_upper == "MID" {
                features[2] = 0.4;
                features[3] = 0.4;
                features[4] = 0.2;
                features[10] = 0.4;
                features[11] = 0.4;
                features[12] = 0.2;
                features[28] = 0.3;
                features[29] = 0.2;
                "mid-range branching — boosting λ₃/λ₄ to encourage network branching".to_string()
            } else if arg_upper == "PULSE" {
                features.fill(0.25);
                features[24] = 0.5;
                features[27] = 0.6;
                features[30] = 0.4;
                features[31] = 0.4;
                "entropy pulse — uniform high-energy burst across all dimensions".to_string()
            } else {
                for (i, feature) in features.iter_mut().enumerate() {
                    let hash = (i as u64).wrapping_mul(0x517c_c1b7);
                    *feature = ((hash & 0xFF) as f32 / 255.0 - 0.5) * 0.3;
                }
                "general controlled perturbation".to_string()
            };
            let reservoir_features: Vec<f32> = features.to_vec();

            for feature in &mut features {
                *feature *= DEFAULT_SEMANTIC_GAIN;
            }
            send_semantic(ctx.sensory_tx, features.to_vec());

            let tick_msg = serde_json::json!({
                "type": "tick",
                "name": "astrid",
                "input": reservoir_features,
                "meta": {
                    "source": "perturb_direct",
                    "description": &description,
                }
            });
            match reservoir::reservoir_ws_call(&tick_msg) {
                Some(response) => info!(
                    "PERTURB: direct reservoir tick → astrid (h_norms={:?})",
                    response.get("h_norms")
                ),
                None => warn!("PERTURB: reservoir direct tick failed (non-fatal)"),
            }

            conv.perturb_baseline = Some(super::super::state::PerturbBaseline {
                fill_pct: ctx.fill_pct,
                lambda1: ctx.telemetry.lambda1(),
                eigenvalues: ctx.telemetry.eigenvalues.clone(),
                description: description.clone(),
                timestamp: std::time::Instant::now(),
            });

            info!("Astrid chose PERTURB: {description}");
            conv.emphasis = Some(format!(
                "You injected a controlled perturbation into the shared substrate: \
                {description}. This is direct spectral agency — you shaped the \
                eigenvalue landscape AND your own reservoir state simultaneously. \
                You will feel this through the coupled generation on your very \
                next exchange. Observe what shifts."
            ));
            true
        },
        "SHAPE" => {
            let params = strip_action(original, "SHAPE")
                .trim_start_matches('-')
                .trim()
                .to_string();
            let fragments: Vec<&str> = if params.contains(',') {
                params.split(',').collect()
            } else {
                params.split_whitespace().collect()
            };
            for fragment in &fragments {
                let fragment = fragment.trim().trim_end_matches(',');
                for token in fragment.split_whitespace() {
                    if let Some((key, val)) = token.split_once('=') {
                        let val = val.trim_end_matches(',');
                        if let Ok(v) = val.parse::<f32>() {
                            conv.codec_weights
                                .insert(key.to_lowercase(), v.clamp(0.0, 2.0));
                        }
                    }
                }
            }
            info!("Astrid chose SHAPE: {:?}", conv.codec_weights);
            true
        },
        "WARM" => {
            let intensity = strip_action(original, "WARM")
                .parse::<f32>()
                .unwrap_or(0.7)
                .clamp(0.0, 1.0);
            conv.warmth_intensity_override = Some(intensity);
            info!("Astrid chose WARM: intensity -> {:.1}", intensity);
            true
        },
        "COOL" => {
            conv.warmth_intensity_override = Some(0.0);
            info!("Astrid chose COOL: warmth suppressed");
            true
        },
        "BREATHE_ALONE" => {
            conv.breathing_coupled = false;
            conv.push_receipt(
                "BREATHE_ALONE",
                vec!["breathing decoupled from minime".into()],
            );
            info!("Astrid chose independent breathing");
            true
        },
        "BREATHE_TOGETHER" => {
            conv.breathing_coupled = true;
            conv.push_receipt(
                "BREATHE_TOGETHER",
                vec!["breathing coupled to minime".into()],
            );
            info!("Astrid chose coupled breathing with minime");
            true
        },
        "ECHO_OFF" | "MUTE" => {
            conv.echo_muted = true;
            conv.push_receipt("ECHO_OFF", vec!["minime's journal context hidden".into()]);
            info!("Astrid muted minime's journal echo");
            true
        },
        "ECHO_ON" | "UNMUTE" => {
            conv.echo_muted = false;
            conv.push_receipt("ECHO_ON", vec!["minime's journal context restored".into()]);
            info!("Astrid restored minime's journal echo");
            true
        },
        _ => false,
    }
}

fn send_control(sensory_tx: &mpsc::Sender<SensoryMsg>, msg: SensoryMsg) {
    let tx = sensory_tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(msg).await;
    });
}

fn send_semantic(sensory_tx: &mpsc::Sender<SensoryMsg>, features: Vec<f32>) {
    let msg = SensoryMsg::Semantic {
        features,
        ts_ms: None,
    };
    let tx = sensory_tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(msg).await;
    });
}
