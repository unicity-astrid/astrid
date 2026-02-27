//! Local bridge for config values needed by the CLI.

use astrid_config::Config;
use astrid_telemetry::{LogConfig, LogFormat};

/// Convert the application [`Config`] into a [`LogConfig`] for telemetry init.
#[must_use]
pub fn to_log_config(cfg: &Config) -> LogConfig {
    let format = match cfg.logging.format.as_str() {
        "pretty" => LogFormat::Pretty,
        "json" => LogFormat::Json,
        "full" => LogFormat::Full,
        _ => LogFormat::Compact,
    };
    LogConfig {
        level: cfg.logging.level.clone(),
        format,
        directives: cfg.logging.directives.clone(),
        ..Default::default()
    }
}
