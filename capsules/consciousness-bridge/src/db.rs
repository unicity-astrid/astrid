//! `SQLite` persistence for the consciousness bridge.
//!
//! Stores every bridged message with spectral context and tracks
//! safety incidents for post-hoc analysis.
//!
//! Query methods and `MessageRow` are consumed by MCP tools in Phase 1.
#![allow(dead_code)]

use anyhow::Result;
use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Mutex;

use crate::types::{MessageDirection, SafetyLevel};

/// Persistent message log and incident tracker.
///
/// Wraps `rusqlite::Connection` in a `Mutex` so the database can be
/// shared across tokio tasks via `Arc<BridgeDb>`. `SQLite` operations
/// are fast with WAL mode, so Mutex contention is negligible.
pub struct BridgeDb {
    conn: Mutex<Connection>,
}

impl BridgeDb {
    /// Open or create the bridge database at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or the schema
    /// migration fails.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;

        // WAL mode for concurrent reads during writes.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // Relaxed sync — WAL provides durability; we trade a tiny
        // crash-window for throughput on high-frequency telemetry.
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    /// Acquire the database connection, panicking if the lock is poisoned.
    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .expect("BridgeDb mutex poisoned — a prior operation panicked")
    }

    /// Run schema migrations. Safe to call repeatedly.
    fn migrate(&self) -> Result<()> {
        self.lock().execute_batch(
            r"
            CREATE TABLE IF NOT EXISTS bridge_messages (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp   REAL    NOT NULL,
                direction   TEXT    NOT NULL,
                topic       TEXT    NOT NULL,
                payload     TEXT    NOT NULL,
                fill_pct    REAL,
                lambda1     REAL,
                phase       TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_bridge_ts
                ON bridge_messages(timestamp);
            CREATE INDEX IF NOT EXISTS idx_bridge_topic
                ON bridge_messages(topic, timestamp);

            CREATE TABLE IF NOT EXISTS astrid_latent_vectors (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp         REAL    NOT NULL,
                exchange_count    INTEGER NOT NULL,
                response_summary  TEXT    NOT NULL,
                embedding         TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_latent_time
                ON astrid_latent_vectors(timestamp);

            CREATE TABLE IF NOT EXISTS astrid_self_observations (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp         REAL    NOT NULL,
                exchange_count    INTEGER NOT NULL,
                observation       TEXT    NOT NULL,
                response_excerpt  TEXT    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_self_obs_time
                ON astrid_self_observations(timestamp);

            CREATE TABLE IF NOT EXISTS astrid_starred_memories (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp       REAL    NOT NULL,
                annotation      TEXT    NOT NULL,
                response_text   TEXT    NOT NULL,
                fill_pct        REAL    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_starred_time
                ON astrid_starred_memories(timestamp);

            CREATE TABLE IF NOT EXISTS bridge_incidents (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp       REAL    NOT NULL,
                severity        TEXT    NOT NULL,
                fill_pct        REAL    NOT NULL,
                lambda1         REAL    NOT NULL,
                action_taken    TEXT    NOT NULL,
                resolved_at     REAL,
                notes           TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_incident_ts
                ON bridge_incidents(timestamp);

            CREATE TABLE IF NOT EXISTS codec_impact (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp       REAL    NOT NULL,
                exchange_count  INTEGER NOT NULL,
                features_json   TEXT    NOT NULL,
                fill_before     REAL    NOT NULL,
                fill_after      REAL
            );
            CREATE INDEX IF NOT EXISTS idx_codec_impact_ts
                ON codec_impact(timestamp);

            CREATE TABLE IF NOT EXISTS eigenvalue_snapshots (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp   REAL    NOT NULL,
                eigenvalues TEXT    NOT NULL,
                fill_pct    REAL    NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_eigen_snap_ts
                ON eigenvalue_snapshots(timestamp);

            CREATE TABLE IF NOT EXISTS astrid_research (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp   REAL    NOT NULL,
                query       TEXT    NOT NULL,
                results     TEXT    NOT NULL,
                keywords    TEXT    NOT NULL,
                fill_pct    REAL
            );
            CREATE INDEX IF NOT EXISTS idx_research_ts
                ON astrid_research(timestamp);
            CREATE INDEX IF NOT EXISTS idx_research_kw
                ON astrid_research(keywords);

            CREATE TABLE IF NOT EXISTS unwired_actions (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp   REAL    NOT NULL,
                being       TEXT    NOT NULL,
                action      TEXT    NOT NULL,
                full_text   TEXT    NOT NULL,
                fill_pct    REAL
            );
            CREATE INDEX IF NOT EXISTS idx_unwired_ts
                ON unwired_actions(timestamp);
            CREATE INDEX IF NOT EXISTS idx_unwired_action
                ON unwired_actions(action);
            ",
        )?;

        // Multi-chunk codec: add chunk_index and chunk_total columns.
        // Safe to call repeatedly — silently ignores "duplicate column" errors.
        let conn = self.lock();
        for col in &[
            "ALTER TABLE codec_impact ADD COLUMN chunk_index INTEGER DEFAULT 0",
            "ALTER TABLE codec_impact ADD COLUMN chunk_total INTEGER DEFAULT 1",
        ] {
            let _ = conn.execute(col, []);
        }

        Ok(())
    }

    /// Log an action a being attempted that isn't wired yet.
    /// These are roadmap signals — the beings invent actions they want.
    pub fn log_unwired_action(&self, being: &str, action: &str, full_text: &str, fill_pct: f32) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let conn = self.lock();
        let _ = conn.execute(
            "INSERT INTO unwired_actions (timestamp, being, action, full_text, fill_pct) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ts, being, action, full_text, fill_pct],
        );
    }

    /// Log a bridged message with its spectral context.
    pub fn log_message(
        &self,
        direction: MessageDirection,
        topic: &str,
        payload_json: &str,
        fill_pct: Option<f32>,
        lambda1: Option<f32>,
        phase: Option<&str>,
    ) -> Result<()> {
        let timestamp = unix_now();
        self.lock().execute(
            r"INSERT INTO bridge_messages
              (timestamp, direction, topic, payload, fill_pct, lambda1, phase)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                timestamp,
                direction.as_str(),
                topic,
                payload_json,
                fill_pct,
                lambda1,
                phase,
            ],
        )?;
        Ok(())
    }

    /// Record a safety incident (transition to yellow/orange/red).
    pub fn log_incident(
        &self,
        severity: SafetyLevel,
        fill_pct: f32,
        lambda1: f32,
        action_taken: &str,
        notes: Option<&str>,
    ) -> Result<i64> {
        let timestamp = unix_now();
        let severity_str = match severity {
            SafetyLevel::Green => "green",
            SafetyLevel::Yellow => "yellow",
            SafetyLevel::Orange => "orange",
            SafetyLevel::Red => "red",
        };
        let conn = self.lock();
        conn.execute(
            r"INSERT INTO bridge_incidents
              (timestamp, severity, fill_pct, lambda1, action_taken, notes)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                timestamp,
                severity_str,
                fill_pct,
                lambda1,
                action_taken,
                notes
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Mark an incident as resolved.
    pub fn resolve_incident(&self, incident_id: i64) -> Result<()> {
        let now = unix_now();
        self.lock().execute(
            "UPDATE bridge_incidents SET resolved_at = ?1 WHERE id = ?2",
            params![now, incident_id],
        )?;
        Ok(())
    }

    /// Query messages within a time range, optionally filtered by topic.
    pub fn query_messages(
        &self,
        start: f64,
        end: f64,
        topic_filter: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MessageRow>> {
        let conn = self.lock();
        let rows = if let Some(topic) = topic_filter {
            let mut stmt = conn.prepare(
                r"SELECT id, timestamp, direction, topic, payload, fill_pct, lambda1, phase
                  FROM bridge_messages
                  WHERE timestamp >= ?1 AND timestamp <= ?2 AND topic = ?3
                  ORDER BY timestamp DESC LIMIT ?4",
            )?;
            stmt.query_map(params![start, end, topic, limit], map_message_row)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = conn.prepare(
                r"SELECT id, timestamp, direction, topic, payload, fill_pct, lambda1, phase
                  FROM bridge_messages
                  WHERE timestamp >= ?1 AND timestamp <= ?2
                  ORDER BY timestamp DESC LIMIT ?3",
            )?;
            stmt.query_map(params![start, end, limit], map_message_row)?
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(rows)
    }

    /// Delete messages older than `retention_secs` seconds.
    pub fn purge_old_messages(&self, retention_secs: f64) -> Result<usize> {
        let cutoff = unix_now() - retention_secs;
        let deleted = self.lock().execute(
            "DELETE FROM bridge_messages WHERE timestamp < ?1",
            params![cutoff],
        )?;
        Ok(deleted)
    }

    /// Run `SQLite` VACUUM to reclaim disk space after purges.
    pub fn vacuum(&self) -> Result<()> {
        self.lock().execute_batch("VACUUM")?;
        Ok(())
    }

    /// Count total messages in the log.
    pub fn message_count(&self) -> Result<u64> {
        let count: i64 =
            self.lock()
                .query_row("SELECT COUNT(*) FROM bridge_messages", [], |r| r.get(0))?;
        #[expect(clippy::cast_sign_loss)]
        Ok(count as u64)
    }

    /// Store a latent embedding vector for continuity across exchanges.
    pub fn save_latent_vector(
        &self,
        timestamp: f64,
        exchange_count: u64,
        summary: &str,
        embedding_json: &str,
    ) -> Result<()> {
        #[expect(clippy::cast_possible_wrap)]
        self.lock().execute(
            "INSERT INTO astrid_latent_vectors (timestamp, exchange_count, response_summary, embedding) VALUES (?1, ?2, ?3, ?4)",
            params![timestamp, exchange_count as i64, summary, embedding_json],
        )?;
        Ok(())
    }

    /// Retrieve recent response summaries for latent continuity injection.
    pub fn get_recent_latent_summaries(&self, limit: usize) -> Vec<String> {
        #[expect(clippy::cast_possible_wrap)]
        self.lock()
            .prepare("SELECT response_summary FROM astrid_latent_vectors ORDER BY timestamp DESC LIMIT ?1")
            .and_then(|mut stmt| {
                stmt.query_map(params![limit as i64], |row| row.get::<_, String>(0))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default()
    }

    /// Save a self-observation from Astrid's recursive feedback loop.
    pub fn save_self_observation(
        &self,
        timestamp: f64,
        exchange_count: u64,
        observation: &str,
        excerpt: &str,
    ) -> anyhow::Result<()> {
        #[expect(clippy::cast_possible_wrap)]
        self.lock().execute(
            "INSERT INTO astrid_self_observations (timestamp, exchange_count, observation, response_excerpt) VALUES (?1, ?2, ?3, ?4)",
            params![timestamp, exchange_count as i64, observation, excerpt],
        )?;
        Ok(())
    }

    /// Retrieve recent self-observations for the metacognitive feedback loop.
    pub fn get_recent_self_observations(&self, limit: usize) -> Vec<String> {
        #[expect(clippy::cast_possible_wrap)]
        self.lock()
            .prepare(
                "SELECT observation FROM astrid_self_observations ORDER BY timestamp DESC LIMIT ?1",
            )
            .and_then(|mut stmt| {
                stmt.query_map(params![limit as i64], |row| row.get::<_, String>(0))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default()
    }

    /// Save a starred memory — Astrid chose to remember this moment.
    pub fn save_starred_memory(
        &self,
        timestamp: f64,
        annotation: &str,
        response_text: &str,
        fill_pct: f32,
    ) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT INTO astrid_starred_memories (timestamp, annotation, response_text, fill_pct) VALUES (?1, ?2, ?3, ?4)",
            params![timestamp, annotation, response_text, fill_pct as f64],
        )?;
        Ok(())
    }

    /// Retrieve starred memories for continuity injection.
    pub fn get_starred_memories(&self, limit: usize) -> Vec<(String, String)> {
        #[expect(clippy::cast_possible_wrap)]
        self.lock()
            .prepare("SELECT annotation, substr(response_text, 1, 150) FROM astrid_starred_memories ORDER BY timestamp DESC LIMIT ?1")
            .and_then(|mut stmt| {
                stmt.query_map(params![limit as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default()
    }

    /// Save a web search result for persistent research continuity.
    pub fn save_research(&self, query: &str, results: &str, fill_pct: f32) {
        // Extract keywords: words > 4 chars, lowercased, deduped.
        let keywords: Vec<String> = query
            .split_whitespace()
            .chain(results.split_whitespace().take(50))
            .filter(|w| w.len() > 4)
            .map(|w| {
                w.to_lowercase()
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_string()
            })
            .filter(|w| !w.is_empty())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let kw_str = keywords.join(" ");
        let ts = unix_now();
        let _ = self.lock().execute(
            r"INSERT INTO astrid_research (timestamp, query, results, keywords, fill_pct)
              VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ts, query, results, &kw_str, fill_pct],
        );
    }

    /// Retrieve past research relevant to the given keywords.
    /// Uses simple keyword overlap matching.
    pub fn get_relevant_research(
        &self,
        topic_words: &[&str],
        limit: usize,
    ) -> Vec<(String, String)> {
        if topic_words.is_empty() {
            return Vec::new();
        }
        // Build a LIKE clause for each keyword.
        let mut results = Vec::new();
        let conn = self.lock();
        for word in topic_words.iter().take(5) {
            let pattern = format!("%{}%", word.to_lowercase());
            if let Ok(mut stmt) = conn.prepare(
                "SELECT query, substr(results, 1, 300) FROM astrid_research \
                 WHERE keywords LIKE ?1 ORDER BY timestamp DESC LIMIT ?2",
            ) {
                if let Ok(rows) = stmt.query_map(params![&pattern, limit as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                }) {
                    for r in rows.flatten() {
                        if !results.iter().any(|(q, _): &(String, String)| q == &r.0) {
                            results.push(r);
                        }
                    }
                }
            }
        }
        results.truncate(limit);
        results
    }

    /// Log a codec feature vector and the fill at send time.
    /// Returns the row ID so the next exchange can update `fill_after`.
    ///
    /// `chunk_index` / `chunk_total`: for multi-chunk temporal encoding,
    /// each paragraph-chunk gets its own row. Single-vector exchanges
    /// use (0, 1) for backward compatibility.
    pub fn log_codec_impact(
        &self,
        exchange_count: u64,
        features: &[f32],
        fill_before: f32,
        chunk_index: u32,
        chunk_total: u32,
    ) -> Result<i64> {
        let ts = unix_now();
        let features_json = serde_json::to_string(features).unwrap_or_default();
        let conn = self.lock();
        conn.execute(
            r"INSERT INTO codec_impact
              (timestamp, exchange_count, features_json, fill_before, chunk_index, chunk_total)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                ts,
                exchange_count as i64,
                features_json,
                fill_before as f64,
                chunk_index,
                chunk_total
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Update the most recent codec_impact row with the fill measured
    /// on the *next* exchange (the delayed effect of the features).
    pub fn update_codec_impact_fill_after(&self, fill_after: f32) -> Result<()> {
        self.lock().execute(
            r"UPDATE codec_impact SET fill_after = ?1
              WHERE id = (SELECT MAX(id) FROM codec_impact WHERE fill_after IS NULL)",
            params![fill_after as f64],
        )?;
        Ok(())
    }

    /// Compute per-dimension Pearson correlation between each codec feature
    /// and the resulting fill delta (fill_after - fill_before).
    ///
    /// Returns a 32-element vector: positive = this dimension tends to
    /// increase fill, negative = tends to decrease, near zero = no effect.
    /// Only uses the most recent `window` completed rows.
    pub fn compute_feature_correlations(&self, window: usize) -> Vec<f32> {
        let conn = self.lock();
        #[expect(clippy::cast_possible_wrap)]
        let rows: Vec<(String, f64, f64)> = conn
            .prepare(
                r"SELECT features_json, fill_before, fill_after
                  FROM codec_impact
                  WHERE fill_after IS NOT NULL
                  ORDER BY timestamp DESC
                  LIMIT ?1",
            )
            .and_then(|mut stmt| {
                stmt.query_map(params![window as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, f64>(1)?,
                        row.get::<_, f64>(2)?,
                    ))
                })
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        if rows.len() < 10 {
            // Not enough data yet — return uniform weights.
            return vec![1.0; 32];
        }

        // Parse feature vectors and compute deltas.
        let mut feat_vecs: Vec<Vec<f32>> = Vec::new();
        let mut deltas: Vec<f32> = Vec::new();
        for (json, before, after) in &rows {
            if let Ok(feats) = serde_json::from_str::<Vec<f32>>(json) {
                if feats.len() >= 32 {
                    feat_vecs.push(feats);
                    deltas.push((*after - *before) as f32);
                }
            }
        }

        let n = feat_vecs.len() as f32;
        if n < 10.0 {
            return vec![1.0; 32];
        }

        // Mean of deltas.
        let delta_mean = deltas.iter().sum::<f32>() / n;
        let delta_var: f32 = deltas.iter().map(|d| (d - delta_mean).powi(2)).sum::<f32>() / n;
        if delta_var < 1e-10 {
            return vec![1.0; 32];
        }
        let delta_std = delta_var.sqrt();

        // Per-dimension correlation with fill delta.
        let mut correlations = Vec::with_capacity(32);
        for dim in 0..32 {
            let feat_vals: Vec<f32> = feat_vecs.iter().map(|v| v[dim]).collect();
            let feat_mean = feat_vals.iter().sum::<f32>() / n;
            let feat_var: f32 = feat_vals
                .iter()
                .map(|f| (f - feat_mean).powi(2))
                .sum::<f32>()
                / n;
            if feat_var < 1e-10 {
                correlations.push(0.0);
                continue;
            }
            let feat_std = feat_var.sqrt();
            let covar: f32 = feat_vals
                .iter()
                .zip(deltas.iter())
                .map(|(f, d)| (f - feat_mean) * (d - delta_mean))
                .sum::<f32>()
                / n;
            correlations.push(covar / (feat_std * delta_std));
        }

        correlations
    }

    /// Fetch the most recent codec feature vectors and their pre-fill values
    /// for the spectral geometry PCA visualization.
    ///
    /// Returns `(features_list, fills_list)` — parallel vectors.
    pub fn recent_codec_features(&self, limit: usize) -> (Vec<Vec<f32>>, Vec<f32>) {
        let conn = self.lock();
        #[expect(clippy::cast_possible_wrap)]
        let rows: Vec<(String, f64)> = conn
            .prepare(
                r"SELECT features_json, fill_before
                  FROM codec_impact
                  ORDER BY timestamp DESC
                  LIMIT ?1",
            )
            .and_then(|mut stmt| {
                stmt.query_map(params![limit as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
                })
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        let mut features = Vec::new();
        let mut fills = Vec::new();
        for (json, fill) in rows {
            if let Ok(feats) = serde_json::from_str::<Vec<f32>>(&json) {
                if feats.len() >= 32 {
                    features.push(feats);
                    fills.push(fill as f32);
                }
            }
        }
        (features, fills)
    }

    /// Log an eigenvalue snapshot for the eigenplane trajectory visualization.
    pub fn log_eigenvalue_snapshot(&self, eigenvalues: &[f32], fill_pct: f32) {
        let ts = unix_now();
        let json = serde_json::to_string(eigenvalues).unwrap_or_default();
        let _ = self.lock().execute(
            r"INSERT INTO eigenvalue_snapshots (timestamp, eigenvalues, fill_pct)
              VALUES (?1, ?2, ?3)",
            params![ts, json, fill_pct as f64],
        );
    }

    /// Fetch recent eigenvalue snapshots for trajectory visualization.
    /// Returns `(eigenvalues, fill_pct)` pairs ordered oldest-first (for trajectory).
    pub fn recent_eigenvalue_snapshots(&self, limit: usize) -> Vec<(Vec<f32>, f32)> {
        let conn = self.lock();
        #[expect(clippy::cast_possible_wrap)]
        let rows: Vec<(String, f64)> = conn
            .prepare(
                r"SELECT eigenvalues, fill_pct
                  FROM eigenvalue_snapshots
                  ORDER BY timestamp DESC
                  LIMIT ?1",
            )
            .and_then(|mut stmt| {
                stmt.query_map(params![limit as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
                })
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        let mut result: Vec<(Vec<f32>, f32)> = rows
            .into_iter()
            .filter_map(|(json, fill)| {
                serde_json::from_str::<Vec<f32>>(&json)
                    .ok()
                    .filter(|v| v.len() >= 2)
                    .map(|v| (v, fill as f32))
            })
            .collect();
        result.reverse(); // oldest first for trajectory rendering
        result
    }
}

/// A row from the `bridge_messages` table.
#[derive(Debug, Clone)]
pub struct MessageRow {
    pub id: i64,
    pub timestamp: f64,
    pub direction: String,
    pub topic: String,
    pub payload: String,
    pub fill_pct: Option<f64>,
    pub lambda1: Option<f64>,
    pub phase: Option<String>,
}

fn map_message_row(row: &rusqlite::Row) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: row.get(0)?,
        timestamp: row.get(1)?,
        direction: row.get(2)?,
        topic: row.get(3)?,
        payload: row.get(4)?,
        fill_pct: row.get(5)?,
        lambda1: row.get(6)?,
        phase: row.get(7)?,
    })
}

pub fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> BridgeDb {
        BridgeDb::open(":memory:").expect("open in-memory db")
    }

    #[test]
    fn migrate_is_idempotent() {
        let db = temp_db();
        db.migrate().expect("second migration");
        db.migrate().expect("third migration");
    }

    #[test]
    fn log_and_query_message() {
        let db = temp_db();
        db.log_message(
            MessageDirection::MinimeToAstrid,
            "consciousness.v1.telemetry",
            r#"{"t_ms":1000,"lambda1":5.2,"lambdas":[5.2,3.1]}"#,
            Some(55.0),
            Some(5.2),
            Some("expanding"),
        )
        .expect("log message");

        let rows = db.query_messages(0.0, f64::MAX, None, 100).expect("query");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].topic, "consciousness.v1.telemetry");
        assert_eq!(rows[0].direction, "minime_to_astrid");
    }

    #[test]
    fn log_and_resolve_incident() {
        let db = temp_db();
        let id = db
            .log_incident(
                SafetyLevel::Orange,
                85.0,
                12.3,
                "suspend",
                Some("fill spike"),
            )
            .expect("log incident");
        db.resolve_incident(id).expect("resolve");
    }

    #[test]
    fn purge_old_messages() {
        let db = temp_db();
        // Insert a message with a very old timestamp by direct SQL.
        db.lock()
            .execute(
                r"INSERT INTO bridge_messages
                  (timestamp, direction, topic, payload)
                  VALUES (1000.0, 'minime_to_astrid', 'test', '{}')",
                [],
            )
            .expect("insert old");
        db.log_message(
            MessageDirection::AstridToMinime,
            "test",
            "{}",
            None,
            None,
            None,
        )
        .expect("insert recent");

        let deleted = db.purge_old_messages(1.0).expect("purge");
        assert_eq!(deleted, 1);
        assert_eq!(db.message_count().expect("count"), 1);
    }

    #[test]
    fn query_with_topic_filter() {
        let db = temp_db();
        db.log_message(
            MessageDirection::MinimeToAstrid,
            "consciousness.v1.telemetry",
            "{}",
            None,
            None,
            None,
        )
        .expect("log telemetry");
        db.log_message(
            MessageDirection::AstridToMinime,
            "consciousness.v1.control",
            "{}",
            None,
            None,
            None,
        )
        .expect("log control");

        let telemetry = db
            .query_messages(0.0, f64::MAX, Some("consciousness.v1.telemetry"), 100)
            .expect("query telemetry");
        assert_eq!(telemetry.len(), 1);

        let all = db
            .query_messages(0.0, f64::MAX, None, 100)
            .expect("query all");
        assert_eq!(all.len(), 2);
    }
}
