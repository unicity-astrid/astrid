//! Spark tool — lets the agent read and evolve its own identity.
//!
//! Two actions:
//! - `read`: Returns the current spark as formatted text.
//! - `evolve`: Merges provided fields into the current spark and writes back.

use serde_json::{Value, json};

use crate::spark::SparkConfig;
use crate::{BuiltinTool, ToolContext, ToolError, ToolResult};

/// Built-in tool for reading and evolving the agent's spark identity.
pub struct SparkTool;

#[async_trait::async_trait]
impl BuiltinTool for SparkTool {
    fn name(&self) -> &'static str {
        "spark"
    }

    fn description(&self) -> &'static str {
        "Read or evolve your identity (spark). Use action=\"read\" to see your current identity, \
         or action=\"evolve\" with field updates to change it. Fields: callsign, class, aura, \
         signal, core. Only provided fields are updated; set a field to \"\" to clear it."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["read", "evolve"],
                    "description": "Action to perform: 'read' to see current identity, 'evolve' to update it."
                },
                "callsign": {
                    "type": "string",
                    "description": "Agent's name (e.g. 'Stellar', 'Nova', 'Orion')."
                },
                "class": {
                    "type": "string",
                    "description": "Role archetype (e.g. 'navigator', 'engineer', 'sentinel')."
                },
                "aura": {
                    "type": "string",
                    "description": "Personality energy (e.g. 'calm', 'sharp', 'warm', 'analytical')."
                },
                "signal": {
                    "type": "string",
                    "description": "Communication style (e.g. 'formal', 'concise', 'casual', 'poetic')."
                },
                "core": {
                    "type": "string",
                    "description": "Soul/philosophy — free-form values, learned patterns, personality depth."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing 'action' field".to_string()))?;

        let spark_file = ctx.spark_file.as_ref().ok_or_else(|| {
            ToolError::Other(
                "Spark is not configured. Set up ~/.astrid/ to enable identity.".to_string(),
            )
        })?;

        match action {
            "read" => {
                let spark = SparkConfig::load_from_file(spark_file).unwrap_or_default();
                Ok(format_spark(&spark))
            },
            "evolve" => {
                // Extract optional fields: None = key absent (don't touch),
                // Some("") = clear, Some(value) = update.
                let callsign = extract_optional_string(&args, "callsign");
                let class = extract_optional_string(&args, "class");
                let aura = extract_optional_string(&args, "aura");
                let signal = extract_optional_string(&args, "signal");
                let core = extract_optional_string(&args, "core");

                if callsign.is_none()
                    && class.is_none()
                    && aura.is_none()
                    && signal.is_none()
                    && core.is_none()
                {
                    return Err(ToolError::InvalidArguments(
                        "evolve requires at least one field to update".to_string(),
                    ));
                }

                // Run the locked read-modify-write cycle on a blocking thread
                // to avoid stalling the Tokio executor with flock(2).
                let spark_file = spark_file.clone();
                let spark =
                    tokio::task::spawn_blocking(move || -> Result<SparkConfig, ToolError> {
                        let _lock = crate::spark::acquire_spark_lock(&spark_file)?;
                        let mut spark =
                            SparkConfig::load_from_file(&spark_file).unwrap_or_default();
                        spark.merge_optional(
                            callsign.as_deref(),
                            class.as_deref(),
                            aura.as_deref(),
                            signal.as_deref(),
                            core.as_deref(),
                        );
                        spark.sanitize();
                        spark.save_to_file(&spark_file)?;
                        Ok(spark)
                    })
                    .await
                    .map_err(|e| ToolError::Other(format!("spawn_blocking failed: {e}")))??;

                Ok(format!(
                    "Spark evolved and saved.\n\n{}",
                    format_spark(&spark)
                ))
            },
            other => Err(ToolError::InvalidArguments(format!(
                "unknown action: '{other}'. Use 'read' or 'evolve'."
            ))),
        }
    }
}

/// Extract an optional string field from JSON args.
///
/// Returns `None` if the key is absent, `Some(value)` if present (including
/// `Some("")` for an explicit empty string — used to clear a field).
fn extract_optional_string(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// Format a spark config as human-readable text.
fn format_spark(spark: &SparkConfig) -> String {
    if spark.is_empty() {
        return "No spark configured. Use evolve to set your identity.".to_string();
    }

    let mut lines = Vec::new();

    if !spark.callsign.is_empty() {
        lines.push(format!("Callsign: {}", spark.callsign));
    }
    if !spark.class.is_empty() {
        lines.push(format!("Class: {}", spark.class));
    }
    if !spark.aura.is_empty() {
        lines.push(format!("Aura: {}", spark.aura));
    }
    if !spark.signal.is_empty() {
        lines.push(format!("Signal: {}", spark.signal));
    }
    if !spark.core.is_empty() {
        lines.push(format!("Core: {}", spark.core));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn test_ctx(spark_file: Option<PathBuf>) -> ToolContext {
        let workspace = PathBuf::from("/tmp/test");
        let cwd = Arc::new(RwLock::new(workspace.clone()));
        ToolContext {
            workspace_root: workspace,
            cwd,
            spark_file,
            subagent_spawner: RwLock::new(None),
        }
    }

    #[tokio::test]
    async fn test_read_empty_spark() {
        let dir = tempfile::tempdir().unwrap();
        let spark_path = dir.path().join("spark.toml");
        let ctx = test_ctx(Some(spark_path));

        let tool = SparkTool;
        let result = tool.execute(json!({"action": "read"}), &ctx).await.unwrap();
        assert!(result.contains("No spark configured"));
    }

    #[tokio::test]
    async fn test_read_existing_spark() {
        let dir = tempfile::tempdir().unwrap();
        let spark_path = dir.path().join("spark.toml");
        std::fs::write(&spark_path, "callsign = \"Nova\"\nclass = \"engineer\"\n").unwrap();

        let ctx = test_ctx(Some(spark_path));
        let tool = SparkTool;
        let result = tool.execute(json!({"action": "read"}), &ctx).await.unwrap();
        assert!(result.contains("Nova"));
        assert!(result.contains("engineer"));
    }

    #[tokio::test]
    async fn test_evolve_creates_spark() {
        let dir = tempfile::tempdir().unwrap();
        let spark_path = dir.path().join("spark.toml");
        let ctx = test_ctx(Some(spark_path.clone()));

        let tool = SparkTool;
        let result = tool
            .execute(
                json!({"action": "evolve", "callsign": "Stellar", "aura": "calm"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.contains("Spark evolved"));
        assert!(result.contains("Stellar"));

        // Verify file was written
        let loaded = SparkConfig::load_from_file(&spark_path).unwrap();
        assert_eq!(loaded.callsign, "Stellar");
        assert_eq!(loaded.aura, "calm");
    }

    #[tokio::test]
    async fn test_evolve_merges_fields() {
        let dir = tempfile::tempdir().unwrap();
        let spark_path = dir.path().join("spark.toml");
        std::fs::write(
            &spark_path,
            "callsign = \"Nova\"\nclass = \"engineer\"\naura = \"sharp\"\n",
        )
        .unwrap();

        let ctx = test_ctx(Some(spark_path.clone()));
        let tool = SparkTool;
        let result = tool
            .execute(
                json!({"action": "evolve", "class": "navigator", "core": "I value clarity."}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.contains("Spark evolved"));

        let loaded = SparkConfig::load_from_file(&spark_path).unwrap();
        assert_eq!(loaded.callsign, "Nova"); // preserved
        assert_eq!(loaded.class, "navigator"); // updated
        assert_eq!(loaded.aura, "sharp"); // preserved
        assert_eq!(loaded.core, "I value clarity."); // added
    }

    #[tokio::test]
    async fn test_evolve_clears_field() {
        let dir = tempfile::tempdir().unwrap();
        let spark_path = dir.path().join("spark.toml");
        std::fs::write(
            &spark_path,
            "callsign = \"Nova\"\nclass = \"engineer\"\naura = \"sharp\"\n",
        )
        .unwrap();

        let ctx = test_ctx(Some(spark_path.clone()));
        let tool = SparkTool;
        // Setting aura to "" should clear it
        tool.execute(json!({"action": "evolve", "aura": ""}), &ctx)
            .await
            .unwrap();

        let loaded = SparkConfig::load_from_file(&spark_path).unwrap();
        assert_eq!(loaded.callsign, "Nova"); // preserved
        assert_eq!(loaded.class, "engineer"); // preserved
        assert!(loaded.aura.is_empty()); // cleared
    }

    #[tokio::test]
    async fn test_evolve_empty_fields_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let spark_path = dir.path().join("spark.toml");
        let ctx = test_ctx(Some(spark_path));

        let tool = SparkTool;
        let result = tool.execute(json!({"action": "evolve"}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_no_spark_file_returns_error() {
        let ctx = test_ctx(None);
        let tool = SparkTool;
        let result = tool.execute(json!({"action": "read"}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_unknown_action_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let spark_path = dir.path().join("spark.toml");
        let ctx = test_ctx(Some(spark_path));

        let tool = SparkTool;
        let result = tool.execute(json!({"action": "delete"}), &ctx).await;
        assert!(result.is_err());
    }
}
