//! Configuration for the Telegram bot.
//!
//! Loads settings from the unified Astralis config system (`~/.astralis/config.toml`)
//! with environment variable fallbacks.

use std::path::Path;

use tracing::{debug, warn};

use crate::error::{TelegramBotError, TelegramResult};

/// Telegram bot configuration.
#[derive(Clone)]
pub struct TelegramConfig {
    /// Telegram Bot API token (from `@BotFather`).
    pub bot_token: String,
    /// `WebSocket` URL for the daemon (e.g. `ws://127.0.0.1:3100`).
    /// If not set, auto-discovers from `~/.astralis/daemon.port`.
    pub daemon_url: Option<String>,
    /// Telegram user IDs allowed to interact with the bot.
    /// Empty means allow all users.
    pub allowed_user_ids: Vec<u64>,
    /// Workspace path to use when creating sessions.
    pub workspace_path: Option<String>,
}

impl std::fmt::Debug for TelegramConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramConfig")
            .field("bot_token", &"[REDACTED]")
            .field("daemon_url", &self.daemon_url)
            .field("allowed_user_ids", &self.allowed_user_ids)
            .field("workspace_path", &self.workspace_path)
            .finish()
    }
}

impl TelegramConfig {
    /// Load configuration from the unified config system, falling back to
    /// environment variables.
    ///
    /// Config file locations (highest priority first):
    /// - `{workspace}/.astralis/config.toml`
    /// - `~/.astralis/config.toml`
    /// - `/etc/astralis/config.toml`
    ///
    /// Environment variable fallbacks:
    /// - `TELEGRAM_BOT_TOKEN` → `bot_token`
    /// - `ASTRALIS_DAEMON_URL` → `daemon_url`
    /// - `TELEGRAM_ALLOWED_USERS` (comma-separated) → `allowed_user_ids`
    /// - `ASTRALIS_WORKSPACE` → `workspace_path`
    pub fn load(workspace_root: Option<&Path>) -> TelegramResult<Self> {
        // Try loading from the unified config system.
        let telegram_section = match astralis_config::Config::load(workspace_root) {
            Ok(resolved) => {
                debug!(
                    files = ?resolved.loaded_files,
                    "loaded config from files"
                );
                resolved.config.telegram
            },
            Err(e) => {
                warn!("failed to load config files, using env vars only: {e}");
                let mut section = astralis_config::TelegramSection::default();
                if let Ok(val) = std::env::var("TELEGRAM_BOT_TOKEN")
                    && !val.is_empty()
                {
                    section.bot_token = Some(val);
                }
                if let Ok(val) = std::env::var("ASTRALIS_DAEMON_URL")
                    && !val.is_empty()
                {
                    section.daemon_url = Some(val);
                }
                if let Ok(val) = std::env::var("ASTRALIS_WORKSPACE")
                    && !val.is_empty()
                {
                    section.workspace_path = Some(val);
                }
                section
            },
        };

        // The unified config system already merges env var fallbacks for
        // bot_token, daemon_url, and workspace_path. We just need to handle
        // TELEGRAM_ALLOWED_USERS separately (comma-separated → Vec<u64>).
        let bot_token = telegram_section.bot_token.ok_or_else(|| {
            TelegramBotError::Config(
                "bot_token is required — set [telegram] bot_token in \
                 ~/.astralis/config.toml or TELEGRAM_BOT_TOKEN env var"
                    .to_owned(),
            )
        })?;

        let mut allowed_user_ids = telegram_section.allowed_user_ids;
        if allowed_user_ids.is_empty()
            && let Ok(val) = std::env::var("TELEGRAM_ALLOWED_USERS")
        {
            allowed_user_ids = val
                .split(',')
                .filter_map(|entry| {
                    let trimmed = entry.trim();
                    if trimmed.is_empty() {
                        return None;
                    }
                    match trimmed.parse::<u64>() {
                        Ok(id) => Some(id),
                        Err(e) => {
                            warn!(
                                value = trimmed,
                                error = %e,
                                "ignoring unparseable entry in TELEGRAM_ALLOWED_USERS"
                            );
                            None
                        },
                    }
                })
                .collect();
        }

        Ok(Self {
            bot_token,
            daemon_url: telegram_section.daemon_url,
            allowed_user_ids,
            workspace_path: telegram_section.workspace_path,
        })
    }

    /// Check if a user ID is allowed.
    pub fn is_user_allowed(&self, user_id: u64) -> bool {
        self.allowed_user_ids.is_empty() || self.allowed_user_ids.contains(&user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build a config without going through env vars.
    fn test_config(allowed: Vec<u64>) -> TelegramConfig {
        TelegramConfig {
            bot_token: "test-token".to_owned(),
            daemon_url: None,
            allowed_user_ids: allowed,
            workspace_path: None,
        }
    }

    #[test]
    fn empty_allowlist_permits_everyone() {
        let cfg = test_config(vec![]);
        assert!(cfg.is_user_allowed(12345));
        assert!(cfg.is_user_allowed(99999));
    }

    #[test]
    fn allowlist_permits_listed_users() {
        let cfg = test_config(vec![100, 200, 300]);
        assert!(cfg.is_user_allowed(100));
        assert!(cfg.is_user_allowed(200));
        assert!(cfg.is_user_allowed(300));
    }

    #[test]
    fn allowlist_denies_unlisted_users() {
        let cfg = test_config(vec![100, 200]);
        assert!(!cfg.is_user_allowed(999));
        assert!(!cfg.is_user_allowed(0));
    }
}
