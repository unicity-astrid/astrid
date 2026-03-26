//! Topic schema catalog for A2UI integration.
//!
//! Maps IPC topics to their schema definitions. Populated at capsule load time
//! from `Capsule.toml` topic declarations. The A2UI bridge (Track 2) reads
//! this catalog to generate schema context for the LLM system prompt.
//!
//! Currently intentionally empty infrastructure — no capsule defines WIT types
//! for IPC events yet. Population happens when capsules define WIT records for
//! their published event types (Phase 3 of the wasmtime migration).

use std::collections::HashMap;

use tokio::sync::RwLock;

use crate::capsule::CapsuleId;

/// Schema metadata for a single IPC topic.
#[derive(Debug, Clone)]
pub struct TopicSchema {
    /// ID of the capsule that owns this topic.
    pub capsule_id: CapsuleId,
    /// Human-readable description of the topic (from `Capsule.toml`).
    pub description: Option<String>,
    /// Schema data (JSON Schema or WIT-derived metadata).
    ///
    /// Currently `None` for all topics — populated when capsules define
    /// WIT records for their IPC payloads.
    pub schema: Option<serde_json::Value>,
}

/// Runtime catalog mapping IPC topics to their schemas.
///
/// Thread-safe (uses `RwLock`) and shared across the runtime via `Arc`.
/// Updated on capsule load/unload.
#[derive(Debug, Default)]
pub struct SchemaCatalog {
    schemas: RwLock<HashMap<String, TopicSchema>>,
}

impl SchemaCatalog {
    /// Create an empty schema catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register topic schemas from a capsule's manifest declarations.
    ///
    /// Called during `WasmEngine::load()`. The `baked_schemas` map contains
    /// JSON Schemas derived from WIT records at install time (keyed by topic
    /// name). Topics without a baked schema are still registered with
    /// `schema: None` so the catalog knows they exist.
    pub async fn register_topics(
        &self,
        capsule_id: &CapsuleId,
        topics: &[crate::manifest::TopicDef],
        baked_schemas: &HashMap<String, serde_json::Value>,
    ) {
        let mut schemas = self.schemas.write().await;
        for topic in topics {
            schemas.insert(
                topic.name.clone(),
                TopicSchema {
                    capsule_id: capsule_id.clone(),
                    description: topic.description.clone(),
                    schema: baked_schemas.get(&topic.name).cloned(),
                },
            );
        }
    }

    /// Unregister all topics owned by a capsule (on unload).
    pub async fn unregister_capsule(&self, capsule_id: &CapsuleId) {
        let mut schemas = self.schemas.write().await;
        schemas.retain(|_, v| &v.capsule_id != capsule_id);
    }

    /// Look up the schema for a specific topic.
    pub async fn get(&self, topic: &str) -> Option<TopicSchema> {
        self.schemas.read().await.get(topic).cloned()
    }

    /// Get all registered topic schemas.
    ///
    /// Used by the A2UI bridge to generate the full schema context
    /// for the LLM system prompt.
    pub async fn all(&self) -> HashMap<String, TopicSchema> {
        self.schemas.read().await.clone()
    }

    /// Number of registered topics.
    pub async fn len(&self) -> usize {
        self.schemas.read().await.len()
    }

    /// Whether the catalog is empty.
    pub async fn is_empty(&self) -> bool {
        self.schemas.read().await.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{TopicDef, TopicDirection};

    fn test_capsule_id() -> CapsuleId {
        CapsuleId::from_static("test-capsule")
    }

    #[tokio::test]
    async fn register_and_lookup() {
        let catalog = SchemaCatalog::new();
        let topics = vec![TopicDef {
            name: "registry.v1.active_model_changed".into(),
            direction: TopicDirection::Publish,
            description: Some("Published when the active model changes".into()),
            schema: None,
            wit_type: None,
        }];

        catalog
            .register_topics(&test_capsule_id(), &topics, &HashMap::new())
            .await;

        let schema = catalog.get("registry.v1.active_model_changed").await;
        assert!(schema.is_some());
        let schema = schema.unwrap();
        assert_eq!(schema.capsule_id, test_capsule_id());
        assert!(schema.description.is_some());
        assert!(schema.schema.is_none());
    }

    #[tokio::test]
    async fn register_with_baked_schema() {
        let catalog = SchemaCatalog::new();
        let topics = vec![TopicDef {
            name: "registry.v1.active_model_changed".into(),
            direction: TopicDirection::Publish,
            description: Some("Published when the active model changes".into()),
            schema: None,
            wit_type: Some("provider-entry".into()),
        }];

        let mut baked = HashMap::new();
        baked.insert(
            "registry.v1.active_model_changed".into(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Model ID"}
                }
            }),
        );

        catalog
            .register_topics(&test_capsule_id(), &topics, &baked)
            .await;

        let schema = catalog.get("registry.v1.active_model_changed").await;
        assert!(schema.is_some());
        let schema = schema.unwrap();
        assert!(schema.schema.is_some());
        let json_schema = schema.schema.unwrap();
        assert_eq!(json_schema["properties"]["id"]["type"], "string");
    }

    #[tokio::test]
    async fn unregister_capsule_removes_its_topics() {
        let catalog = SchemaCatalog::new();
        let id = test_capsule_id();
        let topics = vec![
            TopicDef {
                name: "a.v1.foo".into(),
                direction: TopicDirection::Publish,
                description: None,
                schema: None,
                wit_type: None,
            },
            TopicDef {
                name: "a.v1.bar".into(),
                direction: TopicDirection::Subscribe,
                description: None,
                schema: None,
                wit_type: None,
            },
        ];

        catalog.register_topics(&id, &topics, &HashMap::new()).await;
        assert_eq!(catalog.len().await, 2);

        catalog.unregister_capsule(&id).await;
        assert!(catalog.is_empty().await);
    }

    #[tokio::test]
    async fn multiple_capsules_independent() {
        let catalog = SchemaCatalog::new();
        let id_a = CapsuleId::from_static("capsule-a");
        let id_b = CapsuleId::from_static("capsule-b");

        catalog
            .register_topics(
                &id_a,
                &[TopicDef {
                    name: "a.v1.event".into(),
                    direction: TopicDirection::Publish,
                    description: None,
                    schema: None,
                    wit_type: None,
                }],
                &HashMap::new(),
            )
            .await;

        catalog
            .register_topics(
                &id_b,
                &[TopicDef {
                    name: "b.v1.event".into(),
                    direction: TopicDirection::Publish,
                    description: None,
                    schema: None,
                    wit_type: None,
                }],
                &HashMap::new(),
            )
            .await;

        assert_eq!(catalog.len().await, 2);

        catalog.unregister_capsule(&id_a).await;
        assert_eq!(catalog.len().await, 1);
        assert!(catalog.get("b.v1.event").await.is_some());
        assert!(catalog.get("a.v1.event").await.is_none());
    }
}
