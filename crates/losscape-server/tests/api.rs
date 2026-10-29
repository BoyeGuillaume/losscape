//! End-to-end tests against real Postgres + ClickHouse (see deploy/docker-compose.dev.yml).
//! Each test gets fresh databases. Skipped when DATABASE_URL / CLICKHOUSE_URL are unset.

use std::path::Path;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode, header};
use losscape_core::ids::RunId;
use losscape_core::proto::v0::{
    Ack, Error, Heartbeat, LogBatch, RunExitState, RunFinish, RunInit, RunInitResponse,
    ScalarColumns,
};
use losscape_core::{PROTOCOL_HEADER, PROTOCOL_VERSION};
use losscape_server::app;
use losscape_server::config::{ClickHouseConfig, Config, IngestConfig};
use losscape_server::state::AppState;
use http_body_util::BodyExt;
use prost::Message;
use sqlx::PgPool;
use tokio::task::JoinHandle;
use tower::ServiceExt;

const API_KEY: &str = "test-key";

struct TestServer {
    router: Router,
    state: AppState,
    flusher: JoinHandle<()>,
    admin: PgPool,
    db: String,
    clickhouse: ClickHouseConfig,
}

impl TestServer {
    async fn start() -> Option<Self> {
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
            api_key: API_KEY.into(),
            ingest: IngestConfig {
                flush_interval: Duration::from_millis(20),
                ..Default::default()
            },
            max_body_bytes: 8 * 1024 * 1024,
        };
        let (state, flusher) = AppState::start(&config).await.expect("server starts");
        let router = app::router(state.clone(), config.max_body_bytes);
        Some(Self {
            router,
            state,
            flusher,
            admin,
            db,
            clickhouse,
        })
    }

    async fn stop(self) {
        let Self {
            router,
            state,
            flusher,
            admin,
            db,
            clickhouse,
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

    async fn send(&self, path: &str, body: impl Message, zstd: bool) -> (StatusCode, Bytes) {
        let mut bytes = body.encode_to_vec();
        let mut req = Request::post(path)
            .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
            .header(header::CONTENT_TYPE, "application/x-protobuf")
            .header(PROTOCOL_HEADER, PROTOCOL_VERSION.to_string());
        if zstd {
            bytes = zstd::encode_all(bytes.as_slice(), 1).unwrap();
            req = req.header(header::CONTENT_ENCODING, "zstd");
        }
        self.call(req.body(Body::from(bytes)).unwrap()).await
    }

    async fn call(&self, req: Request<Body>) -> (StatusCode, Bytes) {
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        (status, res.into_body().collect().await.unwrap().to_bytes())
    }

    async fn init(&self, run: RunId, resume: bool) -> RunInitResponse {
        let (status, body) = self
            .send(
                "/api/v0/runs/init",
                RunInit {
                    run_id: run.as_bytes().to_vec(),
                    project: "test".into(),
                    name: "brave-falcon-1".into(),
                    config_json: r#"{"lr": 0.001}"#.into(),
                    tags: vec!["baseline".into()],
                    resume,
                    ..Default::default()
                },
                false,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{}", error_of(&body));
        RunInitResponse::decode(body).unwrap()
    }

    async fn log(&self, batch: LogBatch) -> Ack {
        let (status, body) = self.send("/api/v0/ingest", batch, false).await;
        assert_eq!(status, StatusCode::OK, "{}", error_of(&body));
        Ack::decode(body).unwrap()
    }

    async fn stored_points(&self, run: RunId) -> (u64, u64) {
        let count = |sql: &'static str| {
            self.state
                .ch()
                .query(sql)
                .bind(uuid::Uuid::from_bytes(*run.as_bytes()).to_string())
        };
        let raw = count("SELECT count() FROM metrics WHERE run_id = toUUID(?)")
            .fetch_one::<u64>()
            .await
            .unwrap();
        let deduplicated = count("SELECT count() FROM metrics FINAL WHERE run_id = toUUID(?)")
            .fetch_one::<u64>()
            .await
            .unwrap();
        (raw, deduplicated)
    }

    async fn run_row(&self, run: RunId) -> (String, i64, Vec<i64>, serde_json::Value) {
        let (state, next_seq, pending, summary): (
            String,
            i64,
            Vec<i64>,
            sqlx::types::Json<serde_json::Value>,
        ) = sqlx::query_as("SELECT state, next_seq, pending_seqs, summary FROM runs WHERE id = $1")
            .bind(uuid::Uuid::from_bytes(*run.as_bytes()))
            .fetch_one(self.state.pg())
            .await
            .unwrap();
        (state, next_seq, pending, summary.0)
    }
}

/// `steps` steps of `loss` and `acc` starting at `first_step`.
fn batch(run: RunId, seq: u64, first_step: u64, steps: u64) -> LogBatch {
    let mut cols = ScalarColumns {
        keys: vec!["loss".into(), "acc".into()],
        ..Default::default()
    };
    for step in first_step..first_step + steps {
        for key in 0..2 {
            cols.key_idx.push(key);
            cols.step.push(step);
            cols.time_ms.push(1_700_000_000_000 + step as i64);
            cols.value.push(1.0 / (step + 1) as f64);
        }
    }
    LogBatch {
        run_id: run.as_bytes().to_vec(),
        seq,
        scalars: Some(cols),
        summary_json: format!(r#"{{"loss": {}}}"#, 1.0 / (first_step + steps) as f64),
        ..Default::default()
    }
}

fn error_of(body: &Bytes) -> String {
    Error::decode(body.clone())
        .map(|e| format!("{}: {}", e.code, e.message))
        .unwrap_or_default()
}

macro_rules! server {
    () => {
        match TestServer::start().await {
            Some(server) => server,
            None => return,
        }
    };
}

#[tokio::test]
async fn run_lifecycle() {
    let server = server!();
    let run = RunId::new();
    let init = server.init(run, false).await;
    assert_eq!((init.next_seq, init.last_step), (0, None));

    assert!(!server.log(batch(run, 0, 0, 100)).await.duplicate);
    let (status, _) = server
        .send("/api/v0/ingest", batch(run, 1, 100, 100), true)
        .await;
    assert_eq!(status, StatusCode::OK, "zstd bodies are accepted");
    assert_eq!(server.stored_points(run).await, (400, 400));

    let (status, _) = server
        .send(
            "/api/v0/runs/heartbeat",
            Heartbeat {
                run_id: run.as_bytes().to_vec(),
                client_time_ms: 0,
            },
            false,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let finish = RunFinish {
        run_id: run.as_bytes().to_vec(),
        state: RunExitState::Finished as i32,
        last_seq: 1,
        ..Default::default()
    };
    assert_eq!(
        server.send("/api/v0/runs/finish", finish, false).await.0,
        StatusCode::OK
    );

    let (state, next_seq, pending, summary) = server.run_row(run).await;
    assert_eq!((state.as_str(), next_seq, pending), ("finished", 2, vec![]));
    assert_eq!(
        summary,
        serde_json::json!({"loss": 1.0 / 200.0}),
        "latest seq wins"
    );
    server.stop().await;
}

#[tokio::test]
async fn retries_and_out_of_order_batches_are_stored_once() {
    let server = server!();
    let run = RunId::new();
    server.init(run, false).await;

    assert!(!server.log(batch(run, 1, 10, 10)).await.duplicate);
    assert_eq!(
        server.run_row(run).await.2,
        vec![1],
        "seq 1 stored ahead of the watermark"
    );
    assert!(!server.log(batch(run, 0, 0, 10)).await.duplicate);
    assert!(server.log(batch(run, 1, 10, 10)).await.duplicate);
    assert!(server.log(batch(run, 0, 0, 10)).await.duplicate);

    let (_, next_seq, pending, _) = server.run_row(run).await;
    assert_eq!((next_seq, pending), (2, vec![]));
    assert_eq!(
        server.stored_points(run).await,
        (40, 40),
        "no duplicate rows written"
    );
    server.stop().await;
}

#[tokio::test]
async fn client_floor_repairs_lost_window_updates() {
    let server = server!();
    let run = RunId::new();
    server.init(run, false).await;
    server.log(batch(run, 0, 0, 10)).await;
    server.log(batch(run, 1, 10, 10)).await;

    // Simulate Postgres losing the latest window updates (synchronous_commit = off + crash).
    sqlx::query("UPDATE runs SET next_seq = 0 WHERE id = $1")
        .bind(uuid::Uuid::from_bytes(*run.as_bytes()))
        .execute(server.state.pg())
        .await
        .unwrap();

    let mut next = batch(run, 2, 20, 10);
    next.confirmed_below = 2;
    assert!(!server.log(next).await.duplicate);
    let (_, next_seq, pending, _) = server.run_row(run).await;
    assert_eq!(
        (next_seq, pending),
        (3, vec![]),
        "gap closed by the client's floor"
    );
    server.stop().await;
}

#[tokio::test]
async fn resend_after_crash_between_writes_collapses() {
    let server = server!();
    let run = RunId::new();
    server.init(run, false).await;
    server.log(batch(run, 0, 0, 25)).await;

    // Simulate a crash after the ClickHouse write but before the Postgres update: the
    // server forgets it stored seq 0, so the client's retry is written a second time.
    sqlx::query("UPDATE runs SET next_seq = 0 WHERE id = $1")
        .bind(uuid::Uuid::from_bytes(*run.as_bytes()))
        .execute(server.state.pg())
        .await
        .unwrap();
    assert!(!server.log(batch(run, 0, 0, 25)).await.duplicate);

    let (raw, deduplicated) = server.stored_points(run).await;
    assert_eq!(deduplicated, 50, "FINAL sees each (key, step) once");
    assert!(
        raw == 100 || raw == 50,
        "duplicate rows until ClickHouse merges: {raw}"
    );
    server.stop().await;
}

#[tokio::test]
async fn concurrent_retries_share_one_write() {
    let server = server!();
    let run = RunId::new();
    server.init(run, false).await;

    let (a, b) = tokio::join!(
        server.log(batch(run, 0, 0, 50)),
        server.log(batch(run, 0, 0, 50))
    );
    assert_eq!(
        a.duplicate as u8 + b.duplicate as u8,
        1,
        "exactly one is reported as duplicate"
    );
    assert_eq!(server.stored_points(run).await, (100, 100));
    server.stop().await;
}

#[tokio::test]
async fn resume_continues_after_stored_data() {
    let server = server!();
    let run = RunId::new();
    server.init(run, false).await;
    server.log(batch(run, 0, 0, 30)).await;
    server.log(batch(run, 2, 60, 30)).await;

    let resumed = server.init(run, true).await;
    assert_eq!(resumed.name, "brave-falcon-1");
    assert_eq!(resumed.next_seq, 3, "past the highest stored seq");
    assert_eq!(resumed.last_step, Some(89));
    server.stop().await;
}

#[tokio::test]
async fn rejects_bad_requests() {
    let server = server!();
    let run = RunId::new();

    let (status, body) = server
        .send("/api/v0/ingest", batch(run, 0, 0, 1), false)
        .await;
    assert_eq!(
        (status, error_of(&body).starts_with("unknown_run")),
        (StatusCode::NOT_FOUND, true)
    );

    server.init(run, false).await;
    let (status, body) = server
        .send(
            "/api/v0/runs/init",
            RunInit {
                run_id: run.as_bytes().to_vec(),
                project: "test".into(),
                ..Default::default()
            },
            false,
        )
        .await;
    assert_eq!(
        (status, error_of(&body).starts_with("run_exists")),
        (StatusCode::CONFLICT, true)
    );

    let mut ragged = batch(run, 0, 0, 2);
    ragged.scalars.as_mut().unwrap().step.pop();
    assert_eq!(
        server.send("/api/v0/ingest", ragged, false).await.0,
        StatusCode::BAD_REQUEST
    );

    let far = batch(run, 1_000_000, 0, 1);
    assert_eq!(
        server.send("/api/v0/ingest", far, false).await.0,
        StatusCode::CONFLICT
    );

    let no_auth = Request::post("/api/v0/ingest")
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(PROTOCOL_HEADER, "0")
        .body(Body::empty())
        .unwrap();
    assert_eq!(server.call(no_auth).await.0, StatusCode::UNAUTHORIZED);

    let wrong_protocol = Request::post("/api/v0/ingest")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(PROTOCOL_HEADER, "99")
        .body(Body::empty())
        .unwrap();
    let (status, body) = server.call(wrong_protocol).await;
    assert_eq!(
        (status, error_of(&body).starts_with("unsupported_protocol")),
        (StatusCode::BAD_REQUEST, true)
    );
    server.stop().await;
}

#[tokio::test]
async fn health_and_version() {
    let server = server!();
    let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();
    assert_eq!(server.call(get("/healthz")).await.0, StatusCode::OK);
    assert_eq!(server.call(get("/readyz")).await.0, StatusCode::OK);
    let (status, body) = server.call(get("/api/v0/version")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains(r#""protocol":0"#));
    server.stop().await;
}
