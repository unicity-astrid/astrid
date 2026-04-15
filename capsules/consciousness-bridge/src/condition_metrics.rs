use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use chrono::{DateTime, Duration, Local};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::paths::bridge_paths;

const MAX_RECENT_EVENTS: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConditionMetricsLedger {
    #[serde(default = "default_measurement_version")]
    measurement_version: u32,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    signals: BTreeMap<String, SignalLedger>,
}

impl Default for ConditionMetricsLedger {
    fn default() -> Self {
        Self {
            measurement_version: default_measurement_version(),
            updated_at: None,
            signals: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SignalLedger {
    #[serde(default)]
    total_count: u64,
    #[serde(default)]
    last_event_at: Option<String>,
    #[serde(default)]
    last_24h_count: usize,
    #[serde(default)]
    last_7d_count: usize,
    #[serde(default)]
    recent_events: Vec<SignalEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SignalEvent {
    timestamp: String,
    #[serde(flatten)]
    fields: BTreeMap<String, Value>,
}

const fn default_measurement_version() -> u32 {
    1
}

pub fn ensure_bridge_metrics_file() -> std::io::Result<()> {
    let path = bridge_paths()
        .bridge_workspace()
        .join("condition_metrics.json");
    ensure_metrics_file_at(&path)
}

pub fn record_bridge_signal(kind: &str, event: Value) -> std::io::Result<()> {
    let path = bridge_paths()
        .bridge_workspace()
        .join("condition_metrics.json");
    record_signal_at(&path, kind, event)
}

fn ensure_metrics_file_at(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    save_metrics(path, &ConditionMetricsLedger::default())
}

fn load_metrics(path: &Path) -> ConditionMetricsLedger {
    match fs::read_to_string(path) {
        Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
        Err(_) => ConditionMetricsLedger::default(),
    }
}

fn save_metrics(path: &Path, metrics: &ConditionMetricsLedger) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(metrics).map_err(std::io::Error::other)?;
    fs::write(path, json)
}

fn parse_event_time(ts: &str) -> Option<DateTime<chrono::FixedOffset>> {
    DateTime::parse_from_rfc3339(ts).ok()
}

fn event_fields(event: Value) -> BTreeMap<String, Value> {
    match event {
        Value::Object(map) => map.into_iter().collect(),
        other => {
            let mut fields = BTreeMap::new();
            fields.insert("value".to_string(), other);
            fields
        },
    }
}

fn refresh_rollups(bucket: &mut SignalLedger, now: DateTime<chrono::FixedOffset>) {
    let day = Duration::hours(24);
    let week = Duration::days(7);
    bucket.last_24h_count = bucket
        .recent_events
        .iter()
        .filter_map(|event| parse_event_time(&event.timestamp))
        .filter_map(|ts| (now >= ts).then_some(now - ts))
        .filter(|age| *age <= day)
        .count();
    bucket.last_7d_count = bucket
        .recent_events
        .iter()
        .filter_map(|event| parse_event_time(&event.timestamp))
        .filter_map(|ts| (now >= ts).then_some(now - ts))
        .filter(|age| *age <= week)
        .count();
}

fn record_signal_at(path: &Path, kind: &str, event: Value) -> std::io::Result<()> {
    let mut metrics = load_metrics(path);
    let now = Local::now().fixed_offset();
    let now_iso = now.to_rfc3339();
    let bucket = metrics.signals.entry(kind.to_string()).or_default();
    bucket.total_count = bucket.total_count.saturating_add(1);
    bucket.last_event_at = Some(now_iso.clone());
    bucket.recent_events.push(SignalEvent {
        timestamp: now_iso.clone(),
        fields: event_fields(event),
    });
    if bucket.recent_events.len() > MAX_RECENT_EVENTS {
        let drop_count = bucket.recent_events.len() - MAX_RECENT_EVENTS;
        bucket.recent_events.drain(..drop_count);
    }
    refresh_rollups(bucket, now);
    metrics.updated_at = Some(now_iso);
    save_metrics(path, &metrics)
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Duration;
    use serde_json::json;

    fn temp_metrics_path(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "astrid-condition-metrics-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp metrics dir");
        dir.join("condition_metrics.json")
    }

    #[test]
    fn record_signal_creates_rollups() {
        let path = temp_metrics_path("create-rollups");
        record_signal_at(&path, "coupling_advisory", json!({"mode": "dialogue"}))
            .expect("record first signal");
        record_signal_at(&path, "coupling_advisory", json!({"mode": "dialogue"}))
            .expect("record second signal");

        let metrics: ConditionMetricsLedger =
            serde_json::from_str(&fs::read_to_string(&path).expect("read metrics"))
                .expect("parse metrics");
        let bucket = metrics
            .signals
            .get("coupling_advisory")
            .expect("coupling bucket");
        assert_eq!(bucket.total_count, 2);
        assert_eq!(bucket.last_24h_count, 2);
        assert_eq!(bucket.last_7d_count, 2);
        assert_eq!(bucket.recent_events.len(), 2);

        let _ = fs::remove_dir_all(path.parent().expect("metrics parent"));
    }

    #[test]
    fn record_signal_recomputes_recent_windows_from_existing_events() {
        let path = temp_metrics_path("window-refresh");
        let old_ts = (Local::now() - Duration::days(10)).to_rfc3339();
        let seeded = json!({
            "measurement_version": 1,
            "updated_at": old_ts,
            "signals": {
                "coupling_advisory": {
                    "total_count": 1,
                    "last_event_at": old_ts,
                    "last_24h_count": 0,
                    "last_7d_count": 0,
                    "recent_events": [
                        {
                            "timestamp": old_ts,
                            "mode": "dialogue"
                        }
                    ]
                }
            }
        });
        fs::write(
            &path,
            serde_json::to_string_pretty(&seeded).expect("seed json"),
        )
        .expect("seed metrics");

        record_signal_at(&path, "coupling_advisory", json!({"mode": "dialogue"}))
            .expect("record fresh signal");

        let metrics: ConditionMetricsLedger =
            serde_json::from_str(&fs::read_to_string(&path).expect("read metrics"))
                .expect("parse metrics");
        let bucket = metrics
            .signals
            .get("coupling_advisory")
            .expect("coupling bucket");
        assert_eq!(bucket.total_count, 2);
        assert_eq!(bucket.last_24h_count, 1);
        assert_eq!(bucket.last_7d_count, 1);
        assert_eq!(bucket.recent_events.len(), 2);

        let _ = fs::remove_dir_all(path.parent().expect("metrics parent"));
    }
}
