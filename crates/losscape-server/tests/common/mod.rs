//! Shared harness: the server on fresh databases, called in-process.
#![allow(dead_code)] // each test binary uses a different subset

use std::path::Path;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode, header};
use losscape_core::proto::v0::Error;
use losscape_core::{PROTOCOL_HEADER, PROTOCOL_VERSION};
use losscape_server::app;
use losscape_server::auth::account;
use losscape_server::config::{AuthConfig, ClickHouseConfig, Config, IngestConfig};
use losscape_server::state::AppState;
use http_body_util::BodyExt;
use prost::Message;
use sqlx::PgPool;
use tokio::task::JoinHandle;
use tower::ServiceExt;

pub struct TestServer {
    pub router: Router,
    /// API key of the admin user created at start.
    pub api_key: String,
    pub state: AppState,
    pub flusher: JoinHandle<()>,
    pub admin: PgPool,
    pub db: String,
    pub clickhouse: ClickHouseConfig,
}

impl TestServer {
    pub async fn start() -> Option<Self> {
        Self::start_with(AuthConfig::default()).await
    }

    pub async fn start_with(auth: AuthConfig) -> Option<Self> {
        let mut server = Self::start_bare(auth).await?;
        server.api_key = user(&server.state, "admin").await.1;
        Some(server)
    }

    /// No users yet (`api_key` is empty).
    pub async fn start_bare(auth: AuthConfig) -> Option<Self> {
        dotenvy::from_path(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env")).ok();
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
        let config = Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            database_url: format!("{base}/{db}"),
            clickhouse: clickhouse.clone(),
            auth,
            ingest: IngestConfig {
                flush_interval: Duration::from_millis(20),
                ..Default::default()
            },
            max_body_bytes: 8 * 1024 * 1024,
            web_dir: None,
        };
        let (state, flusher) = AppState::start(&config).await.expect("server starts");
        let router = app::router(state.clone(), config.max_body_bytes, None);
        let api_key = String::new();
        Some(Self {
            router,
            api_key,
            state,
            flusher,
            admin,
            db,
            clickhouse,
        })
    }

    pub fn bearer(&self) -> String {
        format!("Bearer {}", self.api_key)
    }

    pub async fn stop(self) {
        let Self {
            router,
            state,
            flusher,
            admin,
            db,
            clickhouse,
            ..
        } = self;
        let pg = state.pg().clone();
        drop((router, state));
        tokio::time::timeout(Duration::from_secs(10), flusher)
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

    pub async fn send(&self, path: &str, body: impl Message, zstd: bool) -> (StatusCode, Bytes) {
        let mut bytes = body.encode_to_vec();
        let mut req = Request::post(path)
            .header(header::AUTHORIZATION, self.bearer())
            .header(header::CONTENT_TYPE, "application/x-protobuf")
            .header(PROTOCOL_HEADER, PROTOCOL_VERSION.to_string());
        if zstd {
            bytes = zstd::encode_all(bytes.as_slice(), 1).unwrap();
            req = req.header(header::CONTENT_ENCODING, "zstd");
        }
        self.call(req.body(Body::from(bytes)).unwrap()).await
    }

    /// Creates a project as the admin (`POST /api/v0/projects`); returns the status.
    pub async fn create_project(&self, name: &str) -> StatusCode {
        let req = Request::post("/api/v0/projects")
            .header(header::AUTHORIZATION, self.bearer())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::json!({ "name": name }).to_string()))
            .unwrap();
        self.call(req).await.0
    }

    pub async fn call(&self, req: Request<Body>) -> (StatusCode, Bytes) {
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        (status, res.into_body().collect().await.unwrap().to_bytes())
    }
}

pub fn error_of(body: &Bytes) -> String {
    Error::decode(body.clone())
        .map(|e| format!("{}: {}", e.code, e.message))
        .unwrap_or_default()
}

#[macro_export]
macro_rules! server {
    () => {
        match TestServer::start().await {
            Some(server) => server,
            None => return,
        }
    };
}

/// Creates a user with an API key (the first one created is admin).
pub async fn user(state: &AppState, username: &str) -> (i64, String) {
    let id = account::create_user(state, username, None, None, None)
        .await
        .unwrap();
    (
        id,
        account::issue_api_key(state, id, "test").await.unwrap().1,
    )
}
