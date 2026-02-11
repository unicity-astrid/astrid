//! Logging configuration and setup.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{
    EnvFilter,
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

use crate::error::{TelemetryError, TelemetryResult};

/// Helper to convert init errors to our error type.
fn init_err<E: std::fmt::Display>(e: E) -> TelemetryError {
    TelemetryError::InitError(e.to_string())
}

/// File rotation strategy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileRotation {
    /// Rotate daily.
    #[default]
    Daily,
    /// Rotate hourly.
    Hourly,
    /// Rotate every minute (for testing).
    Minutely,
    /// Never rotate.
    Never,
}

/// Log format options.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Human-readable format with colors (default).
    #[default]
    Pretty,
    /// Compact single-line format.
    Compact,
    /// JSON format for structured logging.
    Json,
    /// Full format with all fields.
    Full,
}

/// Log output target.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogTarget {
    /// Log to stdout.
    Stdout,
    /// Log to stderr.
    #[default]
    Stderr,
    /// Log to a file (path to directory, filename prefix).
    File(PathBuf),
}

/// File logging configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLogConfig {
    /// Directory to write log files to.
    pub directory: PathBuf,
    /// File name prefix (e.g., "astralis" produces "astralis.2024-01-15.log").
    #[serde(default = "default_file_prefix")]
    pub prefix: String,
    /// Rotation strategy.
    #[serde(default)]
    pub rotation: FileRotation,
    /// Maximum number of log files to keep (0 = unlimited).
    #[serde(default)]
    pub max_files: usize,
}

fn default_file_prefix() -> String {
    "astralis".to_string()
}

impl Default for FileLogConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::from("logs"),
            prefix: default_file_prefix(),
            rotation: FileRotation::default(),
            max_files: 0,
        }
    }
}

/// Logging configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct LogConfig {
    /// Log level filter (e.g., "info", "debug", "trace").
    #[serde(default = "default_level")]
    pub level: String,
    /// Log format.
    #[serde(default)]
    pub format: LogFormat,
    /// Log target.
    #[serde(default)]
    pub target: LogTarget,
    /// File logging configuration (used when target is File).
    #[serde(default)]
    pub file: FileLogConfig,
    /// Whether to include timestamps.
    #[serde(default = "default_true")]
    pub timestamps: bool,
    /// Whether to include file/line info.
    #[serde(default)]
    pub file_info: bool,
    /// Whether to include thread IDs.
    #[serde(default)]
    pub thread_ids: bool,
    /// Whether to include thread names.
    #[serde(default)]
    pub thread_names: bool,
    /// Whether to include span events.
    #[serde(default)]
    pub span_events: bool,
    /// Whether to use ANSI colors.
    #[serde(default = "default_true")]
    pub ansi: bool,
    /// Directive overrides (e.g., `astralis_mcp=debug`).
    #[serde(default)]
    pub directives: Vec<String>,
}

fn default_level() -> String {
    "info".to_string()
}

fn default_true() -> bool {
    true
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_level(),
            format: LogFormat::default(),
            target: LogTarget::default(),
            file: FileLogConfig::default(),
            timestamps: true,
            file_info: false,
            thread_ids: false,
            thread_names: false,
            span_events: false,
            ansi: true,
            directives: Vec::new(),
        }
    }
}

impl LogConfig {
    /// Create a new log config with the specified level.
    #[must_use]
    pub fn new(level: impl Into<String>) -> Self {
        Self {
            level: level.into(),
            ..Default::default()
        }
    }

    /// Set the log format.
    #[must_use]
    pub fn with_format(mut self, format: LogFormat) -> Self {
        self.format = format;
        self
    }

    /// Set the log target.
    #[must_use]
    pub fn with_target(mut self, target: LogTarget) -> Self {
        self.target = target;
        self
    }

    /// Configure file logging with daily rotation.
    #[must_use]
    pub fn with_file_logging(
        mut self,
        directory: impl Into<PathBuf>,
        prefix: impl Into<String>,
    ) -> Self {
        self.target = LogTarget::File(directory.into());
        self.file.prefix = prefix.into();
        self.file.rotation = FileRotation::Daily;
        // Disable ANSI colors for file output
        self.ansi = false;
        self
    }

    /// Configure file logging with custom rotation.
    #[must_use]
    pub fn with_file_logging_rotation(
        mut self,
        directory: impl Into<PathBuf>,
        prefix: impl Into<String>,
        rotation: FileRotation,
    ) -> Self {
        self.target = LogTarget::File(directory.into());
        self.file.prefix = prefix.into();
        self.file.rotation = rotation;
        // Disable ANSI colors for file output
        self.ansi = false;
        self
    }

    /// Add a directive override.
    #[must_use]
    pub fn with_directive(mut self, directive: impl Into<String>) -> Self {
        self.directives.push(directive.into());
        self
    }

    /// Disable timestamps.
    #[must_use]
    pub fn without_timestamps(mut self) -> Self {
        self.timestamps = false;
        self
    }

    /// Enable file/line info.
    #[must_use]
    pub fn with_file_info(mut self) -> Self {
        self.file_info = true;
        self
    }

    /// Enable span events.
    #[must_use]
    pub fn with_span_events(mut self) -> Self {
        self.span_events = true;
        self
    }

    /// Disable ANSI colors.
    #[must_use]
    pub fn without_ansi(mut self) -> Self {
        self.ansi = false;
        self
    }

    /// Build the env filter from config.
    fn build_filter(&self) -> TelemetryResult<EnvFilter> {
        let mut filter = EnvFilter::try_new(&self.level)
            .map_err(|e| TelemetryError::ConfigError(e.to_string()))?;

        for directive in &self.directives {
            filter = filter.add_directive(directive.parse().map_err(
                |e: tracing_subscriber::filter::ParseError| {
                    TelemetryError::ConfigError(e.to_string())
                },
            )?);
        }

        Ok(filter)
    }

    /// Get span events configuration.
    fn span_events(&self) -> FmtSpan {
        if self.span_events {
            FmtSpan::NEW | FmtSpan::CLOSE
        } else {
            FmtSpan::NONE
        }
    }
}

/// Set up logging with the given configuration.
///
/// # Errors
///
/// Returns an error if the configuration is invalid or logging cannot be initialized.
#[allow(clippy::too_many_lines)]
pub fn setup_logging(config: &LogConfig) -> TelemetryResult<()> {
    let filter = config.build_filter()?;

    match (&config.target, config.format) {
        (LogTarget::Stdout, LogFormat::Json) => {
            setup_json_logging(filter, config, std::io::stdout)?;
        },
        (LogTarget::Stdout, LogFormat::Pretty) => {
            setup_pretty_logging(filter, config, std::io::stdout)?;
        },
        (LogTarget::Stdout, LogFormat::Compact) => {
            setup_compact_logging(filter, config, std::io::stdout)?;
        },
        (LogTarget::Stdout, LogFormat::Full) => {
            setup_full_logging(filter, config, std::io::stdout)?;
        },
        (LogTarget::Stderr, LogFormat::Json) => {
            setup_json_logging(filter, config, std::io::stderr)?;
        },
        (LogTarget::Stderr, LogFormat::Pretty) => {
            setup_pretty_logging(filter, config, std::io::stderr)?;
        },
        (LogTarget::Stderr, LogFormat::Compact) => {
            setup_compact_logging(filter, config, std::io::stderr)?;
        },
        (LogTarget::Stderr, LogFormat::Full) => {
            setup_full_logging(filter, config, std::io::stderr)?;
        },
        (LogTarget::File(dir), format) => {
            // Create the directory if it doesn't exist
            std::fs::create_dir_all(dir).map_err(|e| {
                TelemetryError::ConfigError(format!("failed to create log directory: {e}"))
            })?;

            let rotation = match config.file.rotation {
                FileRotation::Daily => Rotation::DAILY,
                FileRotation::Hourly => Rotation::HOURLY,
                FileRotation::Minutely => Rotation::MINUTELY,
                FileRotation::Never => Rotation::NEVER,
            };

            let appender = RollingFileAppender::new(rotation, dir, &config.file.prefix);

            match format {
                LogFormat::Json => setup_json_logging(filter, config, appender)?,
                LogFormat::Pretty => setup_pretty_logging(filter, config, appender)?,
                LogFormat::Compact => setup_compact_logging(filter, config, appender)?,
                LogFormat::Full => setup_full_logging(filter, config, appender)?,
            }
        },
    }

    Ok(())
}

fn setup_json_logging<W>(filter: EnvFilter, config: &LogConfig, writer: W) -> TelemetryResult<()>
where
    W: for<'a> tracing_subscriber::fmt::MakeWriter<'a> + Send + Sync + 'static,
{
    let layer = fmt::layer()
        .json()
        .with_writer(writer)
        .with_file(config.file_info)
        .with_line_number(config.file_info)
        .with_thread_ids(config.thread_ids)
        .with_thread_names(config.thread_names)
        .with_span_events(config.span_events());

    if config.timestamps {
        tracing_subscriber::registry()
            .with(filter)
            .with(layer)
            .try_init()
            .map_err(init_err)
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(layer.without_time())
            .try_init()
            .map_err(init_err)
    }
}

fn setup_pretty_logging<W>(filter: EnvFilter, config: &LogConfig, writer: W) -> TelemetryResult<()>
where
    W: for<'a> tracing_subscriber::fmt::MakeWriter<'a> + Send + Sync + 'static,
{
    let layer = fmt::layer()
        .pretty()
        .with_writer(writer)
        .with_ansi(config.ansi)
        .with_file(config.file_info)
        .with_line_number(config.file_info)
        .with_thread_ids(config.thread_ids)
        .with_thread_names(config.thread_names)
        .with_span_events(config.span_events());

    if config.timestamps {
        tracing_subscriber::registry()
            .with(filter)
            .with(layer)
            .try_init()
            .map_err(init_err)
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(layer.without_time())
            .try_init()
            .map_err(init_err)
    }
}

fn setup_compact_logging<W>(filter: EnvFilter, config: &LogConfig, writer: W) -> TelemetryResult<()>
where
    W: for<'a> tracing_subscriber::fmt::MakeWriter<'a> + Send + Sync + 'static,
{
    let layer = fmt::layer()
        .compact()
        .with_writer(writer)
        .with_ansi(config.ansi)
        .with_file(config.file_info)
        .with_line_number(config.file_info)
        .with_thread_ids(config.thread_ids)
        .with_thread_names(config.thread_names)
        .with_span_events(config.span_events());

    if config.timestamps {
        tracing_subscriber::registry()
            .with(filter)
            .with(layer)
            .try_init()
            .map_err(init_err)
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(layer.without_time())
            .try_init()
            .map_err(init_err)
    }
}

fn setup_full_logging<W>(filter: EnvFilter, config: &LogConfig, writer: W) -> TelemetryResult<()>
where
    W: for<'a> tracing_subscriber::fmt::MakeWriter<'a> + Send + Sync + 'static,
{
    let layer = fmt::layer()
        .with_writer(writer)
        .with_ansi(config.ansi)
        .with_file(config.file_info)
        .with_line_number(config.file_info)
        .with_thread_ids(config.thread_ids)
        .with_thread_names(config.thread_names)
        .with_span_events(config.span_events());

    if config.timestamps {
        tracing_subscriber::registry()
            .with(filter)
            .with(layer)
            .try_init()
            .map_err(init_err)
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(layer.without_time())
            .try_init()
            .map_err(init_err)
    }
}

/// Set up default logging (info level, stderr, pretty format).
///
/// # Errors
///
/// Returns an error if logging cannot be initialized.
pub fn setup_default_logging() -> TelemetryResult<()> {
    setup_logging(&LogConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_config_default() {
        let config = LogConfig::default();
        assert_eq!(config.level, "info");
        assert_eq!(config.format, LogFormat::Pretty);
        assert!(config.timestamps);
        assert!(config.ansi);
    }

    #[test]
    fn test_log_config_builder() {
        let config = LogConfig::new("debug")
            .with_format(LogFormat::Json)
            .without_timestamps()
            .with_file_info()
            .with_directive("astralis_mcp=trace");

        assert_eq!(config.level, "debug");
        assert_eq!(config.format, LogFormat::Json);
        assert!(!config.timestamps);
        assert!(config.file_info);
        assert_eq!(config.directives, vec!["astralis_mcp=trace"]);
    }

    #[test]
    fn test_log_config_serialization() {
        let config = LogConfig::new("warn").with_format(LogFormat::Compact);

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"level\":\"warn\""));
        assert!(json.contains("\"format\":\"compact\""));

        let parsed: LogConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.level, "warn");
        assert_eq!(parsed.format, LogFormat::Compact);
    }

    #[test]
    fn test_build_filter() {
        let config = LogConfig::new("debug").with_directive("astralis=trace");

        let filter = config.build_filter();
        assert!(filter.is_ok());
    }

    #[test]
    fn test_build_filter_invalid() {
        // EnvFilter is permissive with unknown targets, so we test invalid syntax
        let config = LogConfig::new("debug").with_directive("[invalid=syntax");

        let filter = config.build_filter();
        assert!(filter.is_err());
    }
}
