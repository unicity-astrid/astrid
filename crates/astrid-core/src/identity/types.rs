use chrono::{DateTime, Utc};

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;
/// Astrid-native user identity (spans all frontends).
///
/// This is the canonical identifier for a user across all platforms.
/// The same `AstridUserId` is used whether the user is on Discord,
/// WhatsApp, Telegram, or any other frontend.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AstridUserId {
    /// Unique identifier (UUID)
    pub id: Uuid,
    /// Optional ed25519 public key for signing (32 bytes)
    #[serde(
        serialize_with = "serialize_optional_key",
        deserialize_with = "deserialize_optional_key"
    )]
    pub public_key: Option<[u8; 32]>,
    /// Display name
    pub display_name: Option<String>,
    /// When created
    pub created_at: DateTime<Utc>,
}
impl AstridUserId {
    /// Create a new Astrid user identity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            public_key: None,
            display_name: None,
            created_at: Utc::now(),
        }
    }

    /// Create an identity with a specific UUID.
    #[must_use]
    pub fn from_uuid(id: Uuid) -> Self {
        Self {
            id,
            public_key: None,
            display_name: None,
            created_at: Utc::now(),
        }
    }

    /// Create an identity with a display name.
    #[must_use]
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    /// Set the ed25519 public key for this identity.
    #[must_use]
    pub fn with_public_key(mut self, key: [u8; 32]) -> Self {
        self.public_key = Some(key);
        self
    }

    /// Check if this identity has a registered signing key.
    #[must_use]
    pub fn has_signing_key(&self) -> bool {
        self.public_key.is_some()
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
/// Links a frontend account to an Astrid identity.
///
/// This enables cross-frontend identity - the same user on Discord
/// and WhatsApp will have the same `AstridUserId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontendLink {
    /// The Astrid identity this frontend account is linked to
    pub astrid_id: Uuid,
    /// Which frontend platform
    pub frontend: FrontendType,
    /// Platform-specific user ID (e.g., Discord snowflake, phone number)
    pub frontend_user_id: String,
    /// When this link was created
    pub linked_at: DateTime<Utc>,
    /// How this link was verified
    pub verification_method: LinkVerificationMethod,
    /// Whether this is the primary (first linked) frontend
    pub is_primary: bool,
}
impl FrontendLink {
    /// Create a new frontend link.
    #[must_use]
    pub fn new(
        astrid_id: Uuid,
        frontend: FrontendType,
        frontend_user_id: impl Into<String>,
        verification_method: LinkVerificationMethod,
        is_primary: bool,
    ) -> Self {
        Self {
            astrid_id,
            frontend,
            frontend_user_id: frontend_user_id.into(),
            linked_at: Utc::now(),
            verification_method,
            is_primary,
        }
    }
}
/// How a frontend link was verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkVerificationMethod {
    /// First frontend creates identity (bootstrap)
    InitialCreation,
    /// Code sent to verified frontend, entered in new frontend
    CodeVerification {
        /// The frontend that verified the code
        verified_via: FrontendType,
    },
    /// Admin manually linked the accounts
    AdminLink {
        /// Admin who performed the link
        admin_id: Uuid,
    },
}
/// Pending link verification code.
#[derive(Debug, Clone)]
pub struct PendingLinkCode {
    /// The verification code
    pub code: String,
    /// The Astrid identity being linked
    pub astrid_id: Uuid,
    /// The frontend requesting the link
    pub requesting_frontend: FrontendType,
    /// The frontend user ID on the requesting frontend
    pub requesting_user_id: String,
    /// When this code expires
    pub expires_at: DateTime<Utc>,
}
impl PendingLinkCode {
    /// Check if this code has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}
/// Supported frontend platforms.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrontendType {
    /// Discord bot integration
    Discord,
    /// WhatsApp integration
    WhatsApp,
    /// Telegram bot integration
    Telegram,
    /// Slack integration
    Slack,
    /// Web dashboard
    Web,
    /// Command-line interface
    Cli,
    /// Custom/third-party frontend
    Custom(String),
}
impl FrontendType {
    /// Return a lowercase canonical key for this platform.
    ///
    /// Known variants return their static name; [`Custom`](Self::Custom) values
    /// are lowercased and, if they match a known variant, collapse to that
    /// variant's key (e.g. `Custom("Telegram")` → `"telegram"`). Unknown
    /// platforms are trimmed and lowercased.
    ///
    /// Returns an empty `Cow::Owned("")` for [`Custom`](Self::Custom) values
    /// that are empty or whitespace-only after trimming. Callers constructing
    /// `Custom` directly should validate the inner string is non-empty.
    ///
    /// This should be used instead of raw `PartialEq` when comparing platform
    /// identity across trust boundaries (e.g. WASM guests, MCP plugins).
    #[must_use]
    pub fn canonical_name(&self) -> Cow<'_, str> {
        match self {
            Self::Discord => Cow::Borrowed("discord"),
            Self::WhatsApp => Cow::Borrowed("whatsapp"),
            Self::Telegram => Cow::Borrowed("telegram"),
            Self::Slack => Cow::Borrowed("slack"),
            Self::Web => Cow::Borrowed("web"),
            Self::Cli => Cow::Borrowed("cli"),
            Self::Custom(name) => {
                // Collapse known aliases back to their canonical form.
                let trimmed = name.trim();
                match trimmed {
                    _ if trimmed.eq_ignore_ascii_case("discord") => Cow::Borrowed("discord"),
                    _ if trimmed.eq_ignore_ascii_case("whatsapp") => Cow::Borrowed("whatsapp"),
                    _ if trimmed.eq_ignore_ascii_case("whats_app") => Cow::Borrowed("whatsapp"),
                    _ if trimmed.eq_ignore_ascii_case("telegram") => Cow::Borrowed("telegram"),
                    _ if trimmed.eq_ignore_ascii_case("slack") => Cow::Borrowed("slack"),
                    _ if trimmed.eq_ignore_ascii_case("web") => Cow::Borrowed("web"),
                    _ if trimmed.eq_ignore_ascii_case("cli") => Cow::Borrowed("cli"),
                    _ => Cow::Owned(trimmed.to_ascii_lowercase()),
                }
            },
        }
    }

    /// Returns `true` if `self` and `other` refer to the same logical platform,
    /// normalizing [`Custom`](Self::Custom) aliases and ignoring case.
    ///
    /// # Examples
    ///
    /// ```
    /// use astrid_core::identity::FrontendType;
    ///
    /// assert!(FrontendType::Telegram.is_same_platform(&FrontendType::Custom("telegram".into())));
    /// assert!(FrontendType::Telegram.is_same_platform(&FrontendType::Custom("Telegram".into())));
    /// assert!(FrontendType::Custom("MATRIX".into()).is_same_platform(&FrontendType::Custom("matrix".into())));
    /// assert!(!FrontendType::Discord.is_same_platform(&FrontendType::Telegram));
    /// ```
    #[must_use]
    pub fn is_same_platform(&self, other: &Self) -> bool {
        self.canonical_name() == other.canonical_name()
    }

    /// Normalize this `FrontendType` to its canonical variant.
    ///
    /// Collapses any [`Custom`](Self::Custom) alias of a known variant back to
    /// the concrete enum variant. Unknown custom names are trimmed and
    /// lowercased. Known variants are returned unchanged.
    ///
    /// Use this at trust boundaries (identity stores, deserialization from
    /// external sources) to ensure stored values use the canonical variant.
    ///
    /// # Examples
    ///
    /// ```
    /// use astrid_core::identity::FrontendType;
    ///
    /// assert!(matches!(FrontendType::Custom("telegram".into()).normalize(), FrontendType::Telegram));
    /// assert!(matches!(FrontendType::Custom("DISCORD".into()).normalize(), FrontendType::Discord));
    /// assert!(matches!(FrontendType::Telegram.normalize(), FrontendType::Telegram));
    /// ```
    #[must_use]
    pub fn normalize(self) -> Self {
        match self {
            Self::Custom(ref name) => {
                let trimmed = name.trim();
                if trimmed.eq_ignore_ascii_case("discord") {
                    Self::Discord
                } else if trimmed.eq_ignore_ascii_case("whatsapp")
                    || trimmed.eq_ignore_ascii_case("whats_app")
                {
                    Self::WhatsApp
                } else if trimmed.eq_ignore_ascii_case("telegram") {
                    Self::Telegram
                } else if trimmed.eq_ignore_ascii_case("slack") {
                    Self::Slack
                } else if trimmed.eq_ignore_ascii_case("web") {
                    Self::Web
                } else if trimmed.eq_ignore_ascii_case("cli") {
                    Self::Cli
                } else {
                    Self::Custom(trimmed.to_ascii_lowercase())
                }
            },
            known => known,
        }
    }
}
impl fmt::Display for FrontendType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Discord => write!(f, "discord"),
            Self::WhatsApp => write!(f, "whatsapp"),
            Self::Telegram => write!(f, "telegram"),
            Self::Slack => write!(f, "slack"),
            Self::Web => write!(f, "web"),
            Self::Cli => write!(f, "cli"),
            Self::Custom(name) => write!(f, "custom:{name}"),
        }
    }
}

// Serde's serialize_with requires &Option<T> signature, not Option<&T>
#[allow(clippy::ref_option)]
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

impl PartialEq for FrontendType {
    fn eq(&self, other: &Self) -> bool {
        self.canonical_name() == other.canonical_name()
    }
}

impl Eq for FrontendType {}

impl std::hash::Hash for FrontendType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.canonical_name().hash(state);
    }
}

impl FromStr for FrontendType {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().trim() {
            "discord" => Self::Discord,
            "telegram" => Self::Telegram,
            "whatsapp" | "whats_app" => Self::WhatsApp,
            "slack" => Self::Slack,
            "web" => Self::Web,
            "cli" => Self::Cli,
            other => Self::Custom(other.to_string()),
        })
    }
}

impl fmt::Display for LinkVerificationMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InitialCreation => write!(f, "initial_creation"),
            Self::CodeVerification { verified_via } => write!(f, "code_via:{verified_via}"),
            Self::AdminLink { admin_id } => write!(f, "admin:{}", &admin_id.to_string()[..8]),
        }
    }
}
