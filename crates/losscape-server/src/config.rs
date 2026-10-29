use std::net::SocketAddr;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: SocketAddr,
    pub database_url: String,
    pub clickhouse: ClickHouseConfig,
    /// Bearer token the SDK must send. Replaced by per-user keys once accounts exist.
    pub api_key: String,
    pub ingest: IngestConfig,
    /// Maximum request body size, before decompression.
    pub max_body_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct ClickHouseConfig {
    pub url: String,
    pub user: String,
    pub password: String,
    pub database: String,
}

#[derive(Clone, Debug)]
pub struct IngestConfig {
    /// Longest a batch waits in the queue before being written.
    pub flush_interval: Duration,
    /// Write as soon as this many points are queued.
    pub flush_max_rows: usize,
    /// Batches the queue holds before requests start waiting (backpressure).
    pub queue_capacity: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing environment variable {0}")]
    Missing(&'static str),
    #[error("invalid value for {name}: {reason}")]
    Invalid { name: &'static str, reason: String },
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            listen: parse("LOSSCAPE_LISTEN", "0.0.0.0:8080")?,
            database_url: required("DATABASE_URL")?,
            clickhouse: ClickHouseConfig {
                url: required("CLICKHOUSE_URL")?,
                user: optional("CLICKHOUSE_USER", "default"),
                password: optional("CLICKHOUSE_PASSWORD", ""),
                database: optional("CLICKHOUSE_DATABASE", "losscape"),
            },
            api_key: required("LOSSCAPE_API_KEY")?,
            ingest: IngestConfig {
                flush_interval: Duration::from_millis(parse("LOSSCAPE_FLUSH_INTERVAL_MS", "250")?),
                flush_max_rows: parse("LOSSCAPE_FLUSH_MAX_ROWS", "200000")?,
                queue_capacity: parse("LOSSCAPE_INGEST_QUEUE", "4096")?,
            },
            max_body_bytes: parse::<usize>("LOSSCAPE_MAX_BODY_MB", "32")? * 1024 * 1024,
        })
    }
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            flush_interval: Duration::from_millis(250),
            flush_max_rows: 200_000,
            queue_capacity: 4096,
        }
    }
}

fn required(name: &'static str) -> Result<String, ConfigError> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or(ConfigError::Missing(name))
}

fn optional(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn parse<T>(name: &'static str, default: &str) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    optional(name, default)
        .parse()
        .map_err(|e: T::Err| ConfigError::Invalid {
            name,
            reason: e.to_string(),
        })
}
