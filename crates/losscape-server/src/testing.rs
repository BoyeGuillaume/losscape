//! A real server on fresh, throwaway databases, for end-to-end tests of clients.
//! Needs DATABASE_URL and CLICKHOUSE_URL (e.g. from the repo's `.env`, see
//! deploy/docker-compose.dev.yml); `start` returns `None` without them so tests can skip.

use std::path::Path;
use std::time::Duration;

use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::config::{ClickHouseConfig, Config, IngestConfig};
use crate::state::AppState;

pub const API_KEY: &str = "test-key";

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
            api_key: API_KEY.into(),
            ingest: IngestConfig {
                flush_interval: Duration::from_millis(20),
                ..Default::default()
            },
            max_body_bytes: 32 * 1024 * 1024,
        };
        let (state, flusher) = AppState::start(&config).await.expect("server starts");
        let router = crate::app::router(state.clone(), config.max_body_bytes);
        let (shutdown, stop) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    stop.await.ok();
                })
                .await
                .unwrap();
            flusher.await.unwrap();
        });
        Some(Self {
            url: format!("http://{addr}"),
            api_key: API_KEY.into(),
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
