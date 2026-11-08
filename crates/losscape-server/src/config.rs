//! Server settings: from environment variables, and optionally a TOML file for whichever
//! aren't set (`--config <file>` or `LOSSCAPE_CONFIG`). See docs/DEPLOYMENT.md.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sha2::{Digest, Sha256};

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
    pub branding: Branding,
}

/// What this deployment calls itself, and how it looks.
#[derive(Clone, Debug)]
pub struct Branding {
    pub app_name: String,
    /// Instead of the built-in icon, in browser tabs and next to the name.
    pub favicon: Option<BrandingImage>,
    /// Behind the sign-in and sign-up forms.
    pub auth_background: Option<BrandingImage>,
}

impl Default for Branding {
    fn default() -> Self {
        Self {
            app_name: DEFAULT_APP_NAME.to_owned(),
            favicon: None,
            auth_background: None,
        }
    }
}

const DEFAULT_APP_NAME: &str = "losscape";
const MAX_APP_NAME: usize = 64;
const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;

/// An image file, read at startup so that a wrong path shows right away.
#[derive(Clone)]
pub struct BrandingImage {
    pub bytes: bytes::Bytes,
    pub content_type: &'static str,
    /// Of its content: part of its URL, so a new image shows despite caching.
    pub hash: String,
}

impl std::fmt::Debug for BrandingImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({} bytes)", self.content_type, self.bytes.len())
    }
}

impl BrandingImage {
    fn load(name: &str, path: &str) -> Result<Self, ConfigError> {
        let extension = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let content_type = match extension.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "avif" => "image/avif",
            "gif" => "image/gif",
            "svg" => "image/svg+xml",
            "ico" => "image/x-icon",
            _ => {
                return Err(invalid(
                    name,
                    format!("{path:?}: use a png, jpg, webp, avif, gif, svg or ico file"),
                ));
            }
        };
        let bytes = std::fs::read(path).map_err(|e| invalid(name, format!("{path:?}: {e}")))?;
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            return Err(invalid(
                name,
                format!(
                    "{path:?}: must be 1 byte to {} MB",
                    MAX_IMAGE_BYTES / 1024 / 1024
                ),
            ));
        }
        let hash = Sha256::digest(&bytes)
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect();
        Ok(Self {
            bytes: bytes.into(),
            content_type,
            hash,
        })
    }
}

impl Branding {
    fn load(settings: &Settings) -> Result<Self, ConfigError> {
        let app_name = settings.optional("LOSSCAPE_APP_NAME", DEFAULT_APP_NAME);
        let app_name = app_name.trim();
        if app_name.chars().count() > MAX_APP_NAME || app_name.chars().any(char::is_control) {
            return Err(invalid(
                "LOSSCAPE_APP_NAME",
                format!("at most {MAX_APP_NAME} characters, on one line"),
            ));
        }
        let image = |name: &str| {
            settings
                .get(name)
                .filter(|path| !path.is_empty())
                .map(|path| BrandingImage::load(name, &path))
                .transpose()
        };
        Ok(Self {
            app_name: if app_name.is_empty() {
                DEFAULT_APP_NAME
            } else {
                app_name
            }
            .to_owned(),
            favicon: image("LOSSCAPE_FAVICON")?,
            auth_background: image("LOSSCAPE_AUTH_BACKGROUND")?,
        })
    }
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
    /// Reverse proxies whose `X-Forwarded-For` is believed, to tell clients apart.
    pub trusted_proxies: Vec<IpNet>,
}

/// An address or a network: `10.0.0.1`, `172.30.0.0/16`, `fd00::/8`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpNet {
    addr: IpAddr,
    prefix: u32,
}

impl IpNet {
    pub fn contains(&self, ip: IpAddr) -> bool {
        let keep = |bits: u32, value: u128| {
            let drop = bits - self.prefix;
            value.checked_shr(drop).unwrap_or(0)
        };
        match (self.addr.to_canonical(), ip.to_canonical()) {
            (IpAddr::V4(a), IpAddr::V4(b)) => {
                keep(32, u32::from(a).into()) == keep(32, u32::from(b).into())
            }
            (IpAddr::V6(a), IpAddr::V6(b)) => keep(128, a.into()) == keep(128, b.into()),
            _ => false,
        }
    }
}

impl FromStr for IpNet {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let (addr, prefix) = text
            .split_once('/')
            .map_or((text, None), |(a, p)| (a, Some(p)));
        let addr: IpAddr = addr
            .parse()
            .map_err(|_| format!("{text:?}: not an address"))?;
        let addr = addr.to_canonical();
        let bits = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => bits,
            Some(p) => p
                .parse()
                .ok()
                .filter(|p| *p <= bits)
                .ok_or_else(|| format!("{text:?}: bad network size"))?,
        };
        Ok(Self { addr, prefix })
    }
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

    fn load(settings: &Settings) -> Result<Self, ConfigError> {
        let public_url = settings
            .get("LOSSCAPE_PUBLIC_URL")
            .filter(|v| !v.is_empty())
            .map(|v| v.trim_end_matches('/').to_owned());
        let trusted_proxies = settings
            .optional("LOSSCAPE_TRUSTED_PROXIES", "")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|net| {
                net.parse()
                    .map_err(|e| invalid("LOSSCAPE_TRUSTED_PROXIES", e))
            })
            .collect::<Result<Vec<IpNet>, _>>()?;
        let signup = match settings.optional("LOSSCAPE_SIGNUP", "open").as_str() {
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
        for id in settings
            .optional("LOSSCAPE_OAUTH", "")
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
            let get = |suffix: &str| settings.get(&var(suffix)).filter(|v| !v.is_empty());
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
            trusted_proxies,
        })
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            public_url: None,
            signup: Signup::Open,
            oauth: Vec::new(),
            trusted_proxies: Vec::new(),
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
    #[error("config file {path}: {reason}")]
    File { path: String, reason: String },
}

impl Config {
    /// From the environment, then the file `--config` or `LOSSCAPE_CONFIG` names, if any.
    pub fn load(file: Option<&Path>) -> Result<Self, ConfigError> {
        Self::from_settings(&Settings::load(file)?)
    }

    fn from_settings(s: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            listen: s.parse("LOSSCAPE_LISTEN", "0.0.0.0:8080")?,
            database_url: s.required("DATABASE_URL")?,
            clickhouse: ClickHouseConfig {
                url: s.required("CLICKHOUSE_URL")?,
                user: s.optional("CLICKHOUSE_USER", "default"),
                password: s.optional("CLICKHOUSE_PASSWORD", ""),
                database: s.optional("CLICKHOUSE_DATABASE", "losscape"),
            },
            auth: AuthConfig::load(s)?,
            ingest: IngestConfig {
                flush_interval: Duration::from_millis(
                    s.parse("LOSSCAPE_FLUSH_INTERVAL_MS", "250")?,
                ),
                flush_max_rows: s.parse("LOSSCAPE_FLUSH_MAX_ROWS", "200000")?,
                queue_capacity: s.parse("LOSSCAPE_INGEST_QUEUE", "4096")?,
            },
            max_body_bytes: s.parse::<usize>("LOSSCAPE_MAX_BODY_MB", "32")? * 1024 * 1024,
            web_dir: Some(s.optional("LOSSCAPE_WEB_DIR", "web/build").into())
                .filter(|dir: &std::path::PathBuf| dir.join("index.html").exists()),
            branding: Branding::load(s)?,
        })
    }
}

/// Where the server listens, for `healthcheck`: it needs no more of the settings than this.
pub fn listen_address(file: Option<&Path>) -> Result<SocketAddr, ConfigError> {
    Settings::load(file)?.parse("LOSSCAPE_LISTEN", "0.0.0.0:8080")
}

/// Settings by their environment variable name.
#[derive(Default)]
struct Settings(HashMap<String, String>);

impl Settings {
    fn load(file: Option<&Path>) -> Result<Self, ConfigError> {
        let named = std::env::var_os("LOSSCAPE_CONFIG").map(std::path::PathBuf::from);
        let mut values = match file.or(named.as_deref()) {
            Some(path) => {
                let text = std::fs::read_to_string(path).map_err(|e| ConfigError::File {
                    path: path.display().to_string(),
                    reason: e.to_string(),
                })?;
                from_toml(&text).map_err(|reason| ConfigError::File {
                    path: path.display().to_string(),
                    reason,
                })?
            }
            None => HashMap::new(),
        };
        // The environment wins: secrets can stay out of the file.
        values.extend(std::env::vars());
        Ok(Self(values))
    }

    fn get(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }

    fn required(&self, name: &'static str) -> Result<String, ConfigError> {
        self.get(name)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| ConfigError::Missing(name.into()))
    }

    fn optional(&self, name: &str, default: &str) -> String {
        self.get(name).unwrap_or_else(|| default.to_owned())
    }

    fn parse<T>(&self, name: &'static str, default: &str) -> Result<T, ConfigError>
    where
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        self.optional(name, default)
            .parse()
            .map_err(|e: T::Err| invalid(name, e.to_string()))
    }
}

/// Each key of the file, as the environment variable it stands for.
const TOML_KEYS: &[(&str, &str)] = &[
    ("listen", "LOSSCAPE_LISTEN"),
    ("public_url", "LOSSCAPE_PUBLIC_URL"),
    ("signup", "LOSSCAPE_SIGNUP"),
    ("trusted_proxies", "LOSSCAPE_TRUSTED_PROXIES"),
    ("web_dir", "LOSSCAPE_WEB_DIR"),
    ("app_name", "LOSSCAPE_APP_NAME"),
    ("favicon", "LOSSCAPE_FAVICON"),
    ("auth_background", "LOSSCAPE_AUTH_BACKGROUND"),
    ("max_body_mb", "LOSSCAPE_MAX_BODY_MB"),
    ("database.url", "DATABASE_URL"),
    ("clickhouse.url", "CLICKHOUSE_URL"),
    ("clickhouse.user", "CLICKHOUSE_USER"),
    ("clickhouse.password", "CLICKHOUSE_PASSWORD"),
    ("clickhouse.database", "CLICKHOUSE_DATABASE"),
    ("ingest.flush_interval_ms", "LOSSCAPE_FLUSH_INTERVAL_MS"),
    ("ingest.flush_max_rows", "LOSSCAPE_FLUSH_MAX_ROWS"),
    ("ingest.queue", "LOSSCAPE_INGEST_QUEUE"),
];
const OAUTH_KEYS: &[&str] = &[
    "type",
    "issuer",
    "url",
    "api_url",
    "client_id",
    "client_secret",
    "name",
];

/// Reads the file into settings. A key it doesn't know is an error: a typo shouldn't quietly
/// leave a setting at its default, least of all a security one.
fn from_toml(text: &str) -> Result<HashMap<String, String>, String> {
    fn scalar(key: &str, value: &toml::Value) -> Result<String, String> {
        match value {
            toml::Value::String(s) => Ok(s.clone()),
            toml::Value::Integer(n) => Ok(n.to_string()),
            toml::Value::Boolean(b) => Ok(b.to_string()),
            toml::Value::Array(items) => items
                .iter()
                .map(|item| scalar(key, item))
                .collect::<Result<Vec<_>, _>>()
                .map(|parts| parts.join(",")),
            _ => Err(format!("{key}: expected a value")),
        }
    }
    fn flatten(prefix: &str, table: &toml::Table, out: &mut Vec<(String, toml::Value)>) {
        for (key, value) in table {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match value {
                toml::Value::Table(inner) => flatten(&path, inner, out),
                other => out.push((path, other.clone())),
            }
        }
    }

    let mut table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let mut values = HashMap::new();

    // [oauth.<id>] sections also make up the list of providers.
    if let Some(oauth) = table.remove("oauth") {
        let toml::Value::Table(providers) = oauth else {
            return Err("oauth: expected [oauth.<id>] sections".into());
        };
        let ids: Vec<&str> = providers.keys().map(String::as_str).collect();
        values.insert("LOSSCAPE_OAUTH".to_owned(), ids.join(","));
        for (id, provider) in &providers {
            let toml::Value::Table(provider) = provider else {
                return Err(format!("oauth.{id}: expected a section"));
            };
            let prefix = id.to_uppercase().replace('-', "_");
            for (key, value) in provider {
                if !OAUTH_KEYS.contains(&key.as_str()) {
                    return Err(format!("oauth.{id}.{key}: unknown setting"));
                }
                let name = format!("LOSSCAPE_OAUTH_{prefix}_{}", key.to_uppercase());
                values.insert(name, scalar(key, value)?);
            }
        }
    }

    let mut flat = Vec::new();
    flatten("", &table, &mut flat);
    for (key, value) in flat {
        let Some((_, name)) = TOML_KEYS.iter().find(|(k, _)| *k == key) else {
            return Err(format!("{key}: unknown setting"));
        };
        values.insert((*name).to_owned(), scalar(&key, &value)?);
    }
    Ok(values)
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

fn invalid(name: &str, reason: String) -> ConfigError {
    ConfigError::Invalid {
        name: name.into(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> Settings {
        Settings(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        )
    }

    const REQUIRED: &[(&str, &str)] = &[
        ("DATABASE_URL", "postgres://db"),
        ("CLICKHOUSE_URL", "http://ch"),
    ];

    #[test]
    fn defaults_with_what_is_required() {
        let config = Config::from_settings(&settings(REQUIRED)).unwrap();
        assert_eq!(config.listen.port(), 8080);
        assert_eq!(config.auth.signup, Signup::Open);
        assert!(config.auth.trusted_proxies.is_empty());
        for missing in REQUIRED {
            let rest: Vec<_> = REQUIRED.iter().copied().filter(|p| p != missing).collect();
            let error = Config::from_settings(&settings(&rest)).err().unwrap();
            assert!(error.to_string().contains(missing.0), "{error}");
        }
    }

    #[test]
    fn reads_every_setting_from_toml() {
        let values = from_toml(
            r#"
            listen = "127.0.0.1:9000"
            public_url = "https://losscape.example.com/"
            signup = "closed"
            trusted_proxies = ["10.0.0.0/8", "fd00::1"]
            max_body_mb = 8

            [database]
            url = "postgres://db"

            [clickhouse]
            url = "http://ch"
            user = "u"
            password = "p"
            database = "d"

            [ingest]
            flush_interval_ms = 100
            flush_max_rows = 5
            queue = 6

            [oauth.github]
            client_id = "a"
            client_secret = "b"

            [oauth.my-idp]
            issuer = "https://idp.example.com"
            client_id = "c"
            client_secret = "d"
            name = "Company"
            "#,
        )
        .unwrap();
        let config = Config::from_settings(&Settings(values)).unwrap();
        assert_eq!(config.listen.to_string(), "127.0.0.1:9000");
        assert_eq!(
            config.auth.public_url.as_deref(),
            Some("https://losscape.example.com")
        );
        assert!(config.auth.cookie_secure());
        assert_eq!(config.auth.signup, Signup::Closed);
        assert_eq!(config.auth.trusted_proxies.len(), 2);
        assert_eq!(config.max_body_bytes, 8 * 1024 * 1024);
        assert_eq!(config.clickhouse.password, "p");
        assert_eq!(config.ingest.flush_interval, Duration::from_millis(100));
        assert_eq!(config.ingest.queue_capacity, 6);
        let names: Vec<&str> = config.auth.oauth.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["GitHub", "Company"]);
    }

    #[test]
    fn the_example_file_is_valid() {
        let example = include_str!("../../../deploy/losscape.example.toml");
        let config = Config::from_settings(&Settings(from_toml(example).unwrap())).unwrap();
        assert_eq!(config.listen.port(), 8080);
        assert_eq!(config.ingest.queue_capacity, 4096);
        // With its optional sections turned on as well.
        let all = example
            .replace("\n# [oauth", "\n[oauth")
            .replace("\n# issuer", "\nissuer");
        let all = all
            .replace("\n# client_", "\nclient_")
            .replace("\n# name", "\nname");
        let all = all.replace("\n# public_url", "\npublic_url");
        let all = all.replace("\n# trusted_proxies", "\ntrusted_proxies");
        let config = Config::from_settings(&Settings(from_toml(&all).unwrap())).unwrap();
        assert_eq!(config.auth.oauth.len(), 2);
        assert_eq!(config.auth.trusted_proxies.len(), 1);
    }

    #[test]
    fn a_typo_in_the_file_is_an_error() {
        for bad in [
            "signups = \"closed\"",
            "[clickhouse]\npasword = \"x\"",
            "[oauth.github]\nclient_secret_ = \"x\"",
            "oauth = 3",
            "not toml",
        ] {
            assert!(from_toml(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn branding_defaults_and_validation() {
        let plain = Branding::load(&settings(&[])).unwrap();
        assert_eq!(plain.app_name, "losscape");
        assert!(plain.favicon.is_none() && plain.auth_background.is_none());

        let named = Branding::load(&settings(&[("LOSSCAPE_APP_NAME", "  Lab Tracker ")])).unwrap();
        assert_eq!(named.app_name, "Lab Tracker");
        let empty = Branding::load(&settings(&[("LOSSCAPE_APP_NAME", " ")])).unwrap();
        assert_eq!(empty.app_name, "losscape");
        let long = "x".repeat(65);
        for bad in [long.as_str(), "two\nlines"] {
            assert!(Branding::load(&settings(&[("LOSSCAPE_APP_NAME", bad)])).is_err());
        }

        let dir = std::env::temp_dir().join(format!("losscape-brand-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path.to_str().unwrap().to_owned()
        };
        let (png, svg) = (file("a.PNG", b"one"), file("b.svg", b"<svg/>"));
        let loaded = Branding::load(&settings(&[
            ("LOSSCAPE_FAVICON", &svg),
            ("LOSSCAPE_AUTH_BACKGROUND", &png),
        ]))
        .unwrap();
        let favicon = loaded.favicon.unwrap();
        assert_eq!(favicon.content_type, "image/svg+xml");
        assert_eq!(favicon.hash.len(), 16);
        assert_eq!(loaded.auth_background.unwrap().content_type, "image/png");

        // A mistake shows at startup, naming the setting.
        let (text, nothing) = (file("c.txt", b"no"), file("d.png", b""));
        let huge = file("e.png", &vec![0; MAX_IMAGE_BYTES + 1]);
        let missing = dir.join("missing.png");
        for bad in [&text, &nothing, &huge, missing.to_str().unwrap()] {
            let error = Branding::load(&settings(&[("LOSSCAPE_FAVICON", bad)]))
                .err()
                .unwrap();
            assert!(error.to_string().contains("LOSSCAPE_FAVICON"), "{error}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn networks() {
        let net = |s: &str| s.parse::<IpNet>().unwrap();
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(net("10.0.0.0/8").contains(ip("10.1.2.3")));
        assert!(!net("10.0.0.0/8").contains(ip("11.0.0.1")));
        assert!(net("192.168.1.5").contains(ip("192.168.1.5")));
        assert!(!net("192.168.1.5").contains(ip("192.168.1.6")));
        assert!(net("0.0.0.0/0").contains(ip("8.8.8.8")));
        assert!(net("fd00::/8").contains(ip("fd12::1")));
        assert!(!net("fd00::/8").contains(ip("fe80::1")));
        assert!(net("::/0").contains(ip("2001:db8::1")));
        // IPv4 seen through an IPv6 socket is the same address.
        assert!(net("172.30.42.10").contains(ip("::ffff:172.30.42.10")));
        assert!(!net("10.0.0.0/8").contains(ip("fd00::1")));
        for bad in ["nope", "10.0.0.0/33", "fd00::/129", "10.0.0.0/x"] {
            assert!(bad.parse::<IpNet>().is_err(), "{bad}");
        }
    }
}
