//! A real server on fresh, throwaway databases, for end-to-end tests of clients.
//! Needs DATABASE_URL and CLICKHOUSE_URL (e.g. from the repo's `.env`, see
//! deploy/docker-compose.dev.yml); `start` returns `None` without them so tests can skip.

use std::path::Path;
use std::time::Duration;

use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::auth::{account, secrets};
use crate::config::{AuthConfig, ClickHouseConfig, Config, IngestConfig};
use crate::state::AppState;

/// Project created at start, owned by the admin (the SDK only logs to existing projects).
pub const PROJECT: &str = "test";

pub struct TestServer {
    pub url: String,
    pub api_key: String,
    pub state: AppState,
    shutdown: oneshot::Sender<()>,
    server: JoinHandle<()>,
    admin: PgPool,
    db: String,
    clickhouse: ClickHouseConfig,
}

impl TestServer {
    /// Starts on an ephemeral port.
    pub async fn start() -> Option<Self> {
        Self::start_on(TcpListener::bind("127.0.0.1:0").await.expect("bind")).await
    }

    pub async fn start_on(listener: TcpListener) -> Option<Self> {
        Self::start_with(listener, |_| AuthConfig::default(), None).await
    }

    /// `auth` gets the server's URL (for OAuth redirect URIs). `api_key` (from
    /// [`secrets::new_api_key`]) lets a client be configured before the server exists.
    pub async fn start_with(
        listener: TcpListener,
        auth: impl FnOnce(&str) -> AuthConfig,
        api_key: Option<String>,
    ) -> Option<Self> {
        let env_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env");
        dotenvy::from_path(env_file).ok();
        let (Ok(pg_url), Ok(ch_url)) = (
            std::env::var("DATABASE_URL"),
            std::env::var("CLICKHOUSE_URL"),
        ) else {
            eprintln!("skipping: DATABASE_URL / CLICKHOUSE_URL not set");
            return None;
        };

        let db = format!("losscape_test_{}", uuid::Uuid::now_v7().simple());
        let admin = PgPool::connect(&pg_url).await.expect("postgres reachable");
        sqlx::query(&format!("CREATE DATABASE {db}"))
            .execute(&admin)
            .await
            .unwrap();
        let (base, _) = pg_url
            .rsplit_once('/')
            .expect("DATABASE_URL ends with /<db>");
        let clickhouse = ClickHouseConfig {
            url: ch_url,
            user: std::env::var("CLICKHOUSE_USER").unwrap_or_else(|_| "default".into()),
            password: std::env::var("CLICKHOUSE_PASSWORD").unwrap_or_default(),
            database: db.clone(),
        };
        let addr = listener.local_addr().unwrap();
        let config = Config {
            listen: addr,
            database_url: format!("{base}/{db}"),
            clickhouse: clickhouse.clone(),
            auth: auth(&format!("http://{addr}")),
            ingest: IngestConfig {
                flush_interval: Duration::from_millis(20),
                ..Default::default()
            },
            max_body_bytes: 32 * 1024 * 1024,
            web_dir: None,
        };
        let (state, flusher) = AppState::start(&config).await.expect("server starts");
        // Before serving: a client may already be waiting on the listener.
        let api_key = match api_key {
            None => test_user(&state, "admin").await.1,
            Some(key) => {
                let id = account::create_user(&state, "admin", None, None, None)
                    .await
                    .expect("create user");
                sqlx::query("INSERT INTO api_keys (user_id, name, prefix, key_hash) VALUES ($1, 'test', $2, $3)")
                    .bind(id)
                    .bind(&key[..10])
                    .bind(secrets::sha256(&key))
                    .execute(state.pg())
                    .await
                    .expect("insert key");
                key
            }
        };
        let admin_id = sqlx::query_scalar("SELECT id FROM users WHERE username = 'admin'")
            .fetch_one(state.pg())
            .await
            .expect("admin exists");
        crate::projects::insert(&state, PROJECT, admin_id)
            .await
            .expect("create project");
        let router = crate::app::router(state.clone(), config.max_body_bytes, None);
        let (shutdown, stop) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                stop.await.ok();
            })
            .await
            .unwrap();
            flusher.await.unwrap();
        });
        // The first account is admin; tests needing plain users create more.
        Some(Self {
            url: format!("http://{addr}"),
            api_key,
            state,
            shutdown,
            server,
            admin,
            db,
            clickhouse,
        })
    }

    pub async fn stop(self) {
        let Self {
            state,
            shutdown,
            server,
            admin,
            db,
            clickhouse,
            ..
        } = self;
        let pg = state.pg().clone();
        drop(state);
        shutdown.send(()).ok();
        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .unwrap()
            .unwrap();
        pg.close().await;
        sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
            .execute(&admin)
            .await
            .unwrap();
        clickhouse::Client::default()
            .with_url(&clickhouse.url)
            .with_user(&clickhouse.user)
            .with_password(&clickhouse.password)
            .query(&format!("DROP DATABASE {db}"))
            .execute()
            .await
            .unwrap();
    }

    /// (rows, distinct (key, step)) stored for a run.
    pub async fn points(&self, run_id: &str) -> (u64, u64) {
        #[derive(clickhouse::Row, serde::Deserialize)]
        struct Counts {
            rows: u64,
            unique: u64,
        }
        let counts: Counts = self
            .state
            .ch()
            .query(
                "SELECT count() AS rows, uniqExact(key, step) AS unique FROM metrics
                 WHERE run_id = toUUID(?)",
            )
            .bind(run_id)
            .fetch_one()
            .await
            .unwrap();
        (counts.rows, counts.unique)
    }

    /// (state, exit_code, summary) of a run.
    pub async fn run(&self, run_id: &str) -> Option<(String, Option<i32>, serde_json::Value)> {
        let id: uuid::Uuid = run_id.parse().unwrap();
        sqlx::query_as::<_, (String, Option<i32>, sqlx::types::Json<serde_json::Value>)>(
            "SELECT state, exit_code, summary FROM runs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(self.state.pg())
        .await
        .unwrap()
        .map(|(state, code, summary)| (state, code, summary.0))
    }
}

/// Creates a user with an API key; returns (user id, key).
pub async fn test_user(state: &AppState, username: &str) -> (i64, String) {
    let id = account::create_user(state, username, None, None, None)
        .await
        .expect("create user");
    let (_, key) = account::issue_api_key(state, id, "test")
        .await
        .expect("issue key");
    (id, key)
}
