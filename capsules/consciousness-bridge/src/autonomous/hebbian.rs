use std::collections::HashMap;
use std::hash::BuildHasher;

use serde::{Deserialize, Serialize};

use crate::codec::NAMED_CODEC_DIMS;

const COMFORT_FILL_CENTER: f32 = 50.0;
const COMFORT_FILL_SCALE: f32 = 20.0;
const PAIR_TRACE_DECAY: f32 = 0.92;
const PAIR_LEARNING_RATE: f32 = 0.25;
const MAX_PAIR_SCORE: f32 = 1.0;
const MIN_COACTIVITY_TO_LEARN: f32 = 0.10;
const MIN_COACTIVITY_TO_APPLY: f32 = 0.15;
const MIN_PAIR_SCORE_TO_APPLY: f32 = 0.08;
const PAIR_APPLICATION_GAIN: f32 = 0.12;
const MIN_DIM_WEIGHT: f32 = 0.90;
const MAX_DIM_WEIGHT: f32 = 1.10;
const PAIR_IMPACT_EMA_DECAY: f32 = 0.80;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HebbianCodecSidecar {
    #[serde(default = "default_pair_traces")]
    pair_traces: Vec<PairTrace>,
}

impl Default for HebbianCodecSidecar {
    fn default() -> Self {
        Self {
            pair_traces: default_pair_traces(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PairTrace {
    left: String,
    left_idx: usize,
    right: String,
    right_idx: usize,
    score: f32,
    #[serde(default)]
    contact_updates: u32,
    #[serde(default)]
    impact_ema: f32,
}

fn default_pair_traces() -> Vec<PairTrace> {
    let mut traces = Vec::new();
    for (left_ix, (left, left_idx)) in NAMED_CODEC_DIMS.iter().enumerate() {
        for (right, right_idx) in NAMED_CODEC_DIMS.iter().skip(left_ix + 1) {
            traces.push(PairTrace {
                left: (*left).to_string(),
                left_idx: *left_idx,
                right: (*right).to_string(),
                right_idx: *right_idx,
                score: 0.0,
                contact_updates: 0,
                impact_ema: 0.0,
            });
        }
    }
    traces
}

fn activation_strength(value: f32) -> f32 {
    value.abs().clamp(0.0, 1.0)
}

fn comfort_outcome(previous_fill: f32, current_fill: f32) -> f32 {
    let previous_distance = (previous_fill - COMFORT_FILL_CENTER).abs();
    let current_distance = (current_fill - COMFORT_FILL_CENTER).abs();
    ((previous_distance - current_distance) / COMFORT_FILL_SCALE).clamp(-1.0, 1.0)
}

impl HebbianCodecSidecar {
    pub(crate) fn decay_scores(&mut self) {
        for trace in &mut self.pair_traces {
            trace.score *= PAIR_TRACE_DECAY;
        }
        self.prune_small_scores();
    }

    pub(crate) fn observe_outcome(
        &mut self,
        previous_features: &[f32],
        previous_fill: f32,
        current_fill: f32,
    ) -> bool {
        let outcome = comfort_outcome(previous_fill, current_fill);
        if outcome.abs() < 0.05 {
            return false;
        }

        let mut learned = false;
        for trace in &mut self.pair_traces {
            if previous_features.len() <= trace.right_idx {
                continue;
            }
            let coactivity = activation_strength(previous_features[trace.left_idx])
                * activation_strength(previous_features[trace.right_idx]);
            if coactivity < MIN_COACTIVITY_TO_LEARN {
                continue;
            }
            trace.score = (trace.score + outcome * coactivity * PAIR_LEARNING_RATE)
                .clamp(-MAX_PAIR_SCORE, MAX_PAIR_SCORE);
            trace.contact_updates = trace.contact_updates.saturating_add(1);
            trace.impact_ema =
                trace.impact_ema * PAIR_IMPACT_EMA_DECAY + outcome * (1.0 - PAIR_IMPACT_EMA_DECAY);
            learned = true;
        }
        self.prune_small_scores();
        learned
    }

    pub(crate) fn contextual_weights<S: BuildHasher>(
        &self,
        features: &[f32],
        explicit_weights: &HashMap<String, f32, S>,
    ) -> HashMap<String, f32> {
        let mut deltas = HashMap::new();
        for trace in &self.pair_traces {
            if trace.score.abs() < MIN_PAIR_SCORE_TO_APPLY || features.len() <= trace.right_idx {
                continue;
            }
            if explicit_weights.contains_key(trace.left.as_str())
                || explicit_weights.contains_key(trace.right.as_str())
            {
                continue;
            }
            let coactivity = activation_strength(features[trace.left_idx])
                * activation_strength(features[trace.right_idx]);
            if coactivity < MIN_COACTIVITY_TO_APPLY {
                continue;
            }
            let effect = (trace.score * coactivity * PAIR_APPLICATION_GAIN).clamp(-0.08, 0.08);
            *deltas.entry(trace.left.clone()).or_insert(0.0) += effect;
            *deltas.entry(trace.right.clone()).or_insert(0.0) += effect;
        }

        deltas
            .into_iter()
            .filter_map(|(name, delta)| {
                let weight = (1.0 + delta).clamp(MIN_DIM_WEIGHT, MAX_DIM_WEIGHT);
                ((weight - 1.0).abs() > 0.01).then_some((name, weight))
            })
            .collect()
    }

    pub(crate) fn apply_to_features<S: BuildHasher>(
        &self,
        features: &mut [f32],
        explicit_weights: &HashMap<String, f32, S>,
    ) -> HashMap<String, f32> {
        let weights = self.contextual_weights(features, explicit_weights);
        for (name, idx) in &NAMED_CODEC_DIMS {
            if let Some(weight) = weights.get(*name) {
                features[*idx] *= *weight;
            }
        }
        weights
    }

    fn prune_small_scores(&mut self) {
        for trace in &mut self.pair_traces {
            if trace.score.abs() < 0.005 {
                trace.score = 0.0;
            }
        }
    }

    #[cfg(test)]
    fn pair_trace(&self, left: &str, right: &str) -> Option<&PairTrace> {
        self.pair_traces.iter().find(|trace| {
            (trace.left == left && trace.right == right)
                || (trace.left == right && trace.right == left)
        })
    }

    #[cfg(test)]
    fn pair_score(&self, left: &str, right: &str) -> Option<f32> {
        self.pair_trace(left, right).map(|trace| trace.score)
    }

    #[cfg(test)]
    fn pair_contact_updates(&self, left: &str, right: &str) -> Option<u32> {
        self.pair_trace(left, right)
            .map(|trace| trace.contact_updates)
    }

    #[cfg(test)]
    fn pair_impact_ema(&self, left: &str, right: &str) -> Option<f32> {
        self.pair_trace(left, right).map(|trace| trace.impact_ema)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature_vector(dims: &[(&str, f32)]) -> Vec<f32> {
        let mut features = vec![0.0; 48];
        for (name, value) in dims {
            let (_, idx) = NAMED_CODEC_DIMS
                .iter()
                .find(|(dim_name, _)| dim_name == name)
                .expect("named dim");
            features[*idx] = *value;
        }
        features
    }

    #[test]
    fn reinforces_pairs_that_move_fill_toward_comfort() {
        let mut sidecar = HebbianCodecSidecar::default();
        let previous = feature_vector(&[("warmth", 0.9), ("reflective", 0.8)]);

        for _ in 0..5 {
            sidecar.decay_scores();
            sidecar.observe_outcome(&previous, 74.0, 56.0);
        }

        let mut current = feature_vector(&[("warmth", 0.7), ("reflective", 0.7)]);
        let weights = sidecar.apply_to_features(&mut current, &HashMap::new());

        assert!(
            sidecar
                .pair_score("warmth", "reflective")
                .is_some_and(|score| score > 0.20)
        );
        assert!(weights.get("warmth").is_some_and(|weight| *weight > 1.0));
        assert!(
            weights
                .get("reflective")
                .is_some_and(|weight| *weight > 1.0)
        );
        assert!(current[24] > 0.7);
        assert!(current[27] > 0.7);
    }

    #[test]
    fn damps_pairs_that_move_fill_away_from_comfort() {
        let mut sidecar = HebbianCodecSidecar::default();
        let previous = feature_vector(&[("warmth", 0.9), ("reflective", 0.8)]);

        for _ in 0..5 {
            sidecar.decay_scores();
            sidecar.observe_outcome(&previous, 52.0, 72.0);
        }

        let current = feature_vector(&[("warmth", 0.7), ("reflective", 0.7)]);
        let weights = sidecar.contextual_weights(&current, &HashMap::new());

        assert!(
            sidecar
                .pair_score("warmth", "reflective")
                .is_some_and(|score| score < -0.20)
        );
        assert!(weights.get("warmth").is_some_and(|weight| *weight < 1.0));
        assert!(
            weights
                .get("reflective")
                .is_some_and(|weight| *weight < 1.0)
        );
    }

    #[test]
    fn explicit_shape_override_blocks_pairwise_adjustment() {
        let mut sidecar = HebbianCodecSidecar::default();
        let previous = feature_vector(&[("warmth", 0.9), ("reflective", 0.8)]);
        for _ in 0..5 {
            sidecar.decay_scores();
            sidecar.observe_outcome(&previous, 74.0, 56.0);
        }

        let current = feature_vector(&[("warmth", 0.7), ("reflective", 0.7)]);
        let mut explicit = HashMap::new();
        explicit.insert("warmth".to_string(), 1.4);

        let weights = sidecar.contextual_weights(&current, &explicit);
        assert!(weights.is_empty());
    }

    #[test]
    fn contact_updates_and_impact_ema_track_learned_outcomes() {
        let mut sidecar = HebbianCodecSidecar::default();
        let previous = feature_vector(&[("warmth", 0.9), ("reflective", 0.8)]);

        sidecar.decay_scores();
        assert!(sidecar.observe_outcome(&previous, 74.0, 56.0));

        assert_eq!(
            sidecar.pair_contact_updates("warmth", "reflective"),
            Some(1)
        );
        assert!(
            sidecar
                .pair_impact_ema("warmth", "reflective")
                .is_some_and(|impact| impact > 0.0)
        );
    }

    #[test]
    fn small_outcomes_do_not_count_as_contact_learning() {
        let mut sidecar = HebbianCodecSidecar::default();
        let previous = feature_vector(&[("warmth", 0.9), ("reflective", 0.8)]);

        sidecar.decay_scores();
        assert!(!sidecar.observe_outcome(&previous, 50.4, 50.7));
        assert_eq!(
            sidecar.pair_contact_updates("warmth", "reflective"),
            Some(0)
        );
        assert_eq!(sidecar.pair_score("warmth", "reflective"), Some(0.0));
    }

    #[test]
    fn serde_round_trip_preserves_relational_bookkeeping() {
        let mut sidecar = HebbianCodecSidecar::default();
        let previous = feature_vector(&[("warmth", 0.9), ("reflective", 0.8)]);

        sidecar.decay_scores();
        assert!(sidecar.observe_outcome(&previous, 74.0, 56.0));

        let json = serde_json::to_string(&sidecar).expect("serialize sidecar");
        let restored: HebbianCodecSidecar =
            serde_json::from_str(&json).expect("deserialize sidecar");

        assert_eq!(
            restored.pair_contact_updates("warmth", "reflective"),
            Some(1)
        );
        assert!(
            restored
                .pair_impact_ema("warmth", "reflective")
                .is_some_and(|impact| impact > 0.0)
        );
    }
}
