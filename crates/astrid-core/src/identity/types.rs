use chrono::{DateTime, Utc};

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Astrid-native user identity (spans all platforms).
///
/// This is the canonical identifier for a user across all platforms.
/// The same `AstridUserId` is used whether the user is on Discord,
/// any platform (Discord, Telegram, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(dead_code)]
pub(crate) struct AstridUserId {
    /// Unique identifier (UUID)
    pub(crate) id: Uuid,
    /// Optional ed25519 public key for signing (32 bytes)
    #[serde(
        serialize_with = "serialize_optional_key",
        deserialize_with = "deserialize_optional_key"
    )]
    pub(crate) public_key: Option<[u8; 32]>,
    /// Display name
    pub(crate) display_name: Option<String>,
    /// When created
    pub(crate) created_at: DateTime<Utc>,
}

impl AstridUserId {
    /// Create a new Astrid user identity.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            public_key: None,
            display_name: None,
            created_at: Utc::now(),
        }
    }

    /// Create an identity with a display name.
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }
}

impl Default for AstridUserId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AstridUserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref name) = self.display_name {
            write!(f, "{}({})", name, &self.id.to_string()[..8])
        } else {
            write!(f, "user:{}", &self.id.to_string()[..8])
        }
    }
}

/// Normalize a platform name: trim whitespace, lowercase.
///
/// This is the only normalization needed. Core doesn't know or care
/// what platforms exist - that's the uplink's business.
#[must_use]
pub(crate) fn normalize_platform(name: impl Into<String>) -> String {
    let s = name.into();
    s.trim().to_ascii_lowercase()
}

// Serde's serialize_with requires &Option<T> signature, not Option<&T>
#[expect(clippy::ref_option)]
fn serialize_optional_key<S>(key: &Option<[u8; 32]>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match key {
        Some(bytes) => {
            let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
            serializer.serialize_some(&encoded)
        },
        None => serializer.serialize_none(),
    }
}

fn deserialize_optional_key<'de, D>(deserializer: D) -> Result<Option<[u8; 32]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        Some(s) => {
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &s)
                .map_err(serde::de::Error::custom)?;
            if bytes.len() != 32 {
                return Err(serde::de::Error::custom("public key must be 32 bytes"));
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok(Some(arr))
        },
        None => Ok(None),
    }
}
