use std::net::SocketAddr;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: SocketAddr,
    pub database_url: String,
    pub clickhouse: ClickHouseConfig,
    pub auth: AuthConfig,
    pub ingest: IngestConfig,
    /// Maximum request body size, before decompression.
    pub max_body_bytes: usize,
    /// Built web UI (`web/build`) to serve, if present.
    pub web_dir: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug)]
pub struct ClickHouseConfig {
    pub url: String,
    pub user: String,
    pub password: String,
    pub database: String,
}

#[derive(Clone, Debug)]
pub struct AuthConfig {
    /// Origin users reach the server at, e.g. `https://losscape.example.com`. Needed for
    /// OAuth redirect URIs; an `https` URL also makes session cookies `Secure`.
    pub public_url: Option<String>,
    pub signup: Signup,
    pub oauth: Vec<OAuthProvider>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signup {
    /// Anyone who can reach the server can create an account (the first one is admin).
    Open,
    /// Only the first account can be created; later users need an existing external
    /// identity link or an admin.
    Closed,
}

#[derive(Clone, Debug)]
pub struct OAuthProvider {
    /// URL-safe id, e.g. `google`; part of the redirect URI.
    pub id: String,
    /// Shown on the sign-in button.
    pub name: String,
    pub kind: OAuthKind,
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Clone, Debug)]
pub enum OAuthKind {
    /// Any OpenID Connect provider (Google, Microsoft, GitLab, Keycloak, Authentik, Okta...).
    Oidc { issuer: String },
    /// GitHub (OAuth2 without OIDC). URLs are configurable for GitHub Enterprise.
    GitHub { web_url: String, api_url: String },
}

impl AuthConfig {
    pub fn cookie_secure(&self) -> bool {
        self.public_url
            .as_deref()
            .is_some_and(|url| url.starts_with("https://"))
    }

    fn from_env() -> Result<Self, ConfigError> {
        let public_url = std::env::var("LOSSCAPE_PUBLIC_URL")
            .ok()
            .filter(|v| !v.is_empty())
            .map(|v| v.trim_end_matches('/').to_owned());
        let signup = match optional("LOSSCAPE_SIGNUP", "open").as_str() {
            "open" => Signup::Open,
            "closed" => Signup::Closed,
            other => {
                return Err(invalid(
                    "LOSSCAPE_SIGNUP",
                    format!("{other:?}: use open or closed"),
                ));
            }
        };
        let mut oauth = Vec::new();
        for id in optional("LOSSCAPE_OAUTH", "")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                return Err(invalid(
                    "LOSSCAPE_OAUTH",
                    format!("{id:?}: ids use [a-z0-9-]"),
                ));
            }
            let var = |suffix: &str| {
                format!(
                    "LOSSCAPE_OAUTH_{}_{suffix}",
                    id.to_uppercase().replace('-', "_")
                )
            };
            let get = |suffix: &str| std::env::var(var(suffix)).ok().filter(|v| !v.is_empty());
            let need = |suffix: &str| get(suffix).ok_or_else(|| ConfigError::Missing(var(suffix)));
            let kind = match get("TYPE").as_deref().unwrap_or(if id == "github" {
                "github"
            } else {
                "oidc"
            }) {
                "oidc" => OAuthKind::Oidc {
                    issuer: need("ISSUER")?,
                },
                "github" => OAuthKind::GitHub {
                    web_url: get("URL").unwrap_or_else(|| "https://github.com".into()),
                    api_url: get("API_URL").unwrap_or_else(|| "https://api.github.com".into()),
                },
                other => {
                    return Err(invalid(
                        "LOSSCAPE_OAUTH_*_TYPE",
                        format!("{other:?}: use oidc or github"),
                    ));
                }
            };
            let mut name = if id == "github" {
                "GitHub".to_owned()
            } else {
                id.to_owned()
            };
            if let Some(first) = name.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            oauth.push(OAuthProvider {
                id: id.to_owned(),
                name: get("NAME").unwrap_or(name),
                kind,
                client_id: need("CLIENT_ID")?,
                client_secret: need("CLIENT_SECRET")?,
            });
        }
        if !oauth.is_empty() && public_url.is_none() {
            return Err(ConfigError::Missing(
                "LOSSCAPE_PUBLIC_URL (required for OAuth redirect URIs)".into(),
            ));
        }
        Ok(Self {
            public_url,
            signup,
            oauth,
        })
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            public_url: None,
            signup: Signup::Open,
            oauth: Vec::new(),
        }
    }
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
    Missing(String),
    #[error("invalid value for {name}: {reason}")]
    Invalid { name: String, reason: String },
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
            auth: AuthConfig::from_env()?,
            ingest: IngestConfig {
                flush_interval: Duration::from_millis(parse("LOSSCAPE_FLUSH_INTERVAL_MS", "250")?),
                flush_max_rows: parse("LOSSCAPE_FLUSH_MAX_ROWS", "200000")?,
                queue_capacity: parse("LOSSCAPE_INGEST_QUEUE", "4096")?,
            },
            max_body_bytes: parse::<usize>("LOSSCAPE_MAX_BODY_MB", "32")? * 1024 * 1024,
            web_dir: Some(optional("LOSSCAPE_WEB_DIR", "web/build").into())
                .filter(|dir: &std::path::PathBuf| dir.join("index.html").exists()),
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
        .ok_or_else(|| ConfigError::Missing(name.into()))
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
        .map_err(|e: T::Err| invalid(name, e.to_string()))
}

fn invalid(name: &str, reason: String) -> ConfigError {
    ConfigError::Invalid {
        name: name.into(),
        reason,
    }
}
