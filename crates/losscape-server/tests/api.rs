//! End-to-end tests against real Postgres + ClickHouse (see deploy/docker-compose.dev.yml).
//! Each test gets fresh databases. Skipped when DATABASE_URL / CLICKHOUSE_URL are unset.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use losscape_core::PROTOCOL_HEADER;
use losscape_core::ids::RunId;
use losscape_core::proto::v0::{
    Ack, ConsoleLine, ConsoleStream, Heartbeat, LogBatch, RunEnvironment, RunExitState, RunFinish,
    RunInit, RunInitResponse, ScalarColumns,
};
use http_body_util::BodyExt;
use prost::Message;
use tower::ServiceExt;

mod common;

use common::{TestServer, error_of};

impl TestServer {
    async fn init(&self, run: RunId, resume: bool) -> RunInitResponse {
        // The SDK only logs to existing projects; tests share "test".
        let created = self.create_project("test").await;
        assert!(matches!(
            created,
            StatusCode::CREATED | StatusCode::CONFLICT
        ));
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

fn console(lines: &[(u64, &str)]) -> Vec<ConsoleLine> {
    lines
        .iter()
        .map(|&(line, text)| ConsoleLine {
            line,
            time_ms: 1_700_000_000_000 + line as i64,
            stream: if text.contains("warn") {
                ConsoleStream::Stderr
            } else {
                ConsoleStream::Stdout
            } as i32,
            text: text.into(),
        })
        .collect()
}

#[tokio::test]
async fn console_lines_are_replaced_and_read_from_the_end() {
    let server = server!();
    let run = RunId::new();
    let id = uuid::Uuid::from_bytes(*run.as_bytes()).simple().to_string();
    assert_eq!(server.create_project("test").await, StatusCode::CREATED);
    let init = RunInit {
        run_id: run.as_bytes().to_vec(),
        project: "test".into(),
        environment: Some(RunEnvironment {
            hostname: "gpu-node-1".into(),
            git_commit: "abc".into(),
            details_json: r#"{"packages": ["torch==2.5.1"], "hostname": "ignored"}"#.into(),
            git_diff: "diff --git a/train.py b/train.py\n+lr = 3e-4\n".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let (status, body) = server.send("/api/v0/runs/init", init.clone(), false).await;
    assert_eq!(status, StatusCode::OK, "{}", error_of(&body));

    let mut first = batch(run, 0, 0, 1);
    first.console = console(&[
        (0, "epoch 1"),
        (1, "\x1b[31mERROR\x1b[0m: disk almost full"),
        (2, "  0%|"),
        (3, "warning: slow"),
    ]);
    server.log(first).await;
    let mut second = batch(run, 1, 1, 1);
    // The progress bar (line 2) is sent again; new lines follow.
    second.console = console(&[(2, "100%|"), (4, "epoch 2"), (5, "done")]);
    // System metrics use unix seconds as steps; resuming must ignore them.
    second
        .scalars
        .as_mut()
        .unwrap()
        .keys
        .push("system/cpu/util_pct".into());
    let s = second.scalars.as_mut().unwrap();
    s.key_idx.push(2);
    s.step.push(1_700_000_000);
    s.time_ms.push(1_700_000_000_000);
    s.value.push(12.0);
    server.log(second).await;

    let texts = |logs: &serde_json::Value| -> Vec<(u64, String)> {
        logs["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| {
                (
                    l["line"].as_u64().unwrap(),
                    l["text"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    let (status, tail) = get_json(&server, &format!("/api/v0/runs/{id}/logs?tail=2")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tail["count"], 6);
    assert_eq!(texts(&tail), [(4, "epoch 2".into()), (5, "done".into())]);

    let (_, page) = get_json(&server, &format!("/api/v0/runs/{id}/logs?from=1&to=4")).await;
    assert_eq!(
        texts(&page),
        [
            (1, "\x1b[31mERROR\x1b[0m: disk almost full".into()),
            (2, "100%|".into()),
            (3, "warning: slow".into())
        ],
        "the redrawn line keeps only its last version"
    );
    assert_eq!(page["lines"][2]["stream"], "stderr");
    assert_eq!(page["lines"][0]["ts_ms"], 1_700_000_000_001i64);

    // Search ignores colors: "ERROR: disk" spans an escape sequence.
    let (_, found) = get_json(&server, &format!("/api/v0/runs/{id}/logs?q=error:%20DISK")).await;
    assert_eq!(texts(&found).iter().map(|l| l.0).collect::<Vec<_>>(), [1]);

    let (_, detail) = get_json(&server, &format!("/api/v0/runs/{id}")).await;
    assert_eq!(
        detail["environment"]["hostname"], "gpu-node-1",
        "details can't override"
    );
    assert_eq!(detail["environment"]["packages"][0], "torch==2.5.1");
    assert_eq!(detail["environment"]["git"]["diff_bytes"], 44);
    let req = Request::get(format!("/api/v0/runs/{id}/files/git.diff"))
        .header(header::AUTHORIZATION, server.bearer())
        .body(Body::empty())
        .unwrap();
    let (status, diff) = server.call(req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&diff).contains("+lr = 3e-4"));

    let resumed = RunInit {
        resume: true,
        ..init
    };
    let (_, body) = server.send("/api/v0/runs/init", resumed, false).await;
    let resumed = RunInitResponse::decode(body).unwrap();
    assert_eq!(resumed.next_console_line, 6);
    assert_eq!(resumed.last_step, Some(1), "system metrics ignored");
    let (_, detail) = get_json(&server, &format!("/api/v0/runs/{id}")).await;
    assert_eq!(
        detail["last_step"], 1,
        "system metrics don't count as steps"
    );
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
        .header(header::AUTHORIZATION, server.bearer())
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

async fn get_json(server: &TestServer, uri: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::get(uri)
        .header(header::AUTHORIZATION, server.bearer())
        .body(Body::empty())
        .unwrap();
    let (status, body) = server.call(req).await;
    (status, serde_json::from_slice(&body).unwrap_or_default())
}

async fn tiles(server: &TestServer, ranges: serde_json::Value) -> Vec<serde_json::Value> {
    let req = Request::post("/api/v0/series?format=json")
        .header(header::AUTHORIZATION, server.bearer())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "ranges": ranges }).to_string(),
        ))
        .unwrap();
    let (status, body) = server.call(req).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    String::from_utf8(body.to_vec())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn read_api_lists_projects_runs_and_metrics() {
    let server = server!();
    let run = RunId::new();
    let id = uuid::Uuid::from_bytes(*run.as_bytes()).simple().to_string();
    server.init(run, false).await;
    server.log(batch(run, 0, 0, 5000)).await;

    let (status, projects) = get_json(&server, "/api/v0/projects").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(projects[0]["name"], "test");
    assert_eq!(projects[0]["runs"], 1);

    let (_, runs) = get_json(&server, "/api/v0/projects/test/runs").await;
    assert_eq!(runs[0]["name"], "brave-falcon-1");
    assert_eq!(runs[0]["state"], "running");
    assert_eq!(runs[0]["config"]["lr"], 0.001);
    let (status, _) = get_json(&server, "/api/v0/projects/nope/runs").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    assert_eq!(runs[0]["aliases"], serde_json::json!([]));
    assert_eq!(runs[0]["last_step"], 4999, "kept by ingest");

    // Older runs get it from ClickHouse at startup.
    sqlx::query("UPDATE runs SET last_step = NULL")
        .execute(server.state.pg())
        .await
        .unwrap();
    assert_eq!(
        losscape_server::purge::backfill_last_step(&server.state)
            .await
            .unwrap(),
        1
    );
    let (_, runs) = get_json(&server, "/api/v0/projects/test/runs").await;
    assert_eq!(runs[0]["last_step"], 4999);
    assert!(runs[0]["start_ms"].as_i64().unwrap() > 0);

    // Metrics of several runs at once; ids of other projects or unknown runs are ignored.
    let req = Request::post("/api/v0/projects/test/metrics")
        .header(header::AUTHORIZATION, server.bearer())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({"runs": [id, RunId::new().to_string()]}).to_string(),
        ))
        .unwrap();
    let (status, body) = server.call(req).await;
    assert_eq!(status, StatusCode::OK);
    let metrics: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        metrics,
        serde_json::json!([
            {"run": uuid::Uuid::from_bytes(*run.as_bytes()).to_string(), "key": "acc", "points": 5000, "min_step": 0, "max_step": 4999},
            {"run": uuid::Uuid::from_bytes(*run.as_bytes()).to_string(), "key": "loss", "points": 5000, "min_step": 0, "max_step": 4999},
        ])
    );

    // Aliases: set, moved by setting them on another run, removed.
    let alias = |method: &str, name: &str, run: Option<&str>| {
        Request::builder()
            .method(method)
            .uri(format!("/api/v0/projects/test/aliases/{name}"))
            .header(header::AUTHORIZATION, server.bearer())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                run.map(|r| serde_json::json!({ "run": r }).to_string())
                    .unwrap_or_default(),
            ))
            .unwrap()
    };
    let other = RunId::new();
    server.init(other, false).await;
    let other_id = uuid::Uuid::from_bytes(*other.as_bytes())
        .simple()
        .to_string();
    for (run, expected) in [
        (&id, StatusCode::NO_CONTENT),
        (&other_id, StatusCode::NO_CONTENT),
    ] {
        assert_eq!(
            server.call(alias("PUT", "best", Some(run))).await.0,
            expected
        );
    }
    assert_eq!(
        server.call(alias("PUT", "@bad", Some(&id))).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        server
            .call(alias("PUT", "best", Some(&RunId::new().to_string())))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (_, runs) = get_json(&server, "/api/v0/projects/test/runs").await;
    let aliases_of = |runs: &serde_json::Value, id: &str| {
        runs.as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"].as_str().unwrap().replace('-', "") == id)
            .unwrap()["aliases"]
            .clone()
    };
    assert_eq!(aliases_of(&runs, &id), serde_json::json!([]), "moved");
    assert_eq!(aliases_of(&runs, &other_id), serde_json::json!(["best"]));
    let (status, target) = get_json(&server, "/api/v0/projects/test/aliases/best").await;
    assert_eq!(
        (status, target["run"].as_str().unwrap().replace('-', "")),
        (StatusCode::OK, other_id.clone())
    );
    assert_eq!(
        server.call(alias("DELETE", "best", None)).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        server.call(alias("DELETE", "best", None)).await.0,
        StatusCode::NOT_FOUND
    );

    let (_, detail) = get_json(&server, &format!("/api/v0/runs/{id}")).await;
    assert_eq!(detail["tags"], serde_json::json!(["baseline"]));
    let metrics = detail["metrics"].as_array().unwrap();
    assert_eq!(metrics.len(), 2);
    assert_eq!(metrics[1]["key"], "loss");
    assert_eq!(
        (
            metrics[1]["points"].as_u64(),
            metrics[1]["max_step"].as_u64()
        ),
        (Some(5000), Some(4999))
    );
    server.stop().await;
}

#[tokio::test]
async fn tiles_aggregate_buckets_and_skip_nan() {
    let server = server!();
    let run = RunId::new();
    let id = uuid::Uuid::from_bytes(*run.as_bytes()).simple().to_string();
    server.init(run, false).await;
    let mut b = batch(run, 0, 0, 5000);
    // loss at step 10 (row 20: two keys per step) is NaN.
    b.scalars.as_mut().unwrap().value[20] = f64::NAN;
    server.log(b).await;

    // Level 0: one bucket per step; tile 1 covers steps 1024..2048.
    let raw = tiles(
        &server,
        serde_json::json!([{"run": id, "key": "loss", "level": 0, "from_tile": 1, "to_tile": 2}]),
    )
    .await;
    assert_eq!(raw.len(), 1024);
    assert_eq!(raw[0]["bucket"].as_f64(), Some(1024.0));
    assert_eq!(raw[0]["v_last"].as_f64(), Some(1.0 / 1025.0));

    // Level 4: 16 steps per bucket, so 5000 steps -> 313 buckets in tile 0.
    let rows = tiles(
        &server,
        serde_json::json!([
            {"run": id, "key": "loss", "level": 4, "from_tile": 0, "to_tile": 1},
            {"run": id, "key": "acc", "level": 12, "from_tile": 0, "to_tile": 1},
        ]),
    )
    .await;
    let (loss, acc): (Vec<_>, Vec<_>) = rows.iter().partition(|r| r["req"] == 0);
    assert_eq!(loss.len(), 313);
    let f = |row: &serde_json::Value, col: &str| row[col].as_f64().unwrap();
    let first = loss[0];
    assert_eq!(first["n"], 15, "NaN at step 10 is not counted");
    assert_eq!((f(first, "v_min"), f(first, "v_max")), (1.0 / 16.0, 1.0));
    assert_eq!((f(first, "v_first"), f(first, "v_last")), (1.0, 1.0 / 16.0));
    assert_eq!(acc.len(), 2, "level 12: 4096 steps per bucket");
    assert_eq!(acc[0]["n"], 4096);

    assert_eq!(
        f(first, "t_mean"),
        1_700_000_000_007.5,
        "mean time of steps 0-15"
    );

    // The web UI reads fixed 64-byte little-endian rows (web/src/lib/charts/tiles.svelte.ts).
    let req = Request::post("/api/v0/series?format=rowbinary")
        .header(header::AUTHORIZATION, server.bearer())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({"ranges": [{"run": id, "key": "loss", "level": 4, "from_tile": 0, "to_tile": 1}]})
                .to_string(),
        ))
        .unwrap();
    let (status, body) = server.call(req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.len(), 313 * 64);
    let row = &body[..64];
    let f64_at = |at: usize| f64::from_le_bytes(row[at..at + 8].try_into().unwrap());
    assert_eq!(u32::from_le_bytes(row[12..16].try_into().unwrap()), 15, "n");
    assert_eq!(
        (f64_at(4), f64_at(16), f64_at(24), f64_at(48), f64_at(56)),
        (0.0, 1.0 / 16.0, 1.0, 1.0 / 16.0, 1_700_000_000_007.5)
    );
    server.stop().await;
}

#[tokio::test]
async fn live_pushes_stored_points_and_state() {
    let server = server!();
    let run = RunId::new();
    let id = uuid::Uuid::from_bytes(*run.as_bytes()).simple().to_string();
    server.init(run, false).await;

    let other = RunId::new();
    server.init(other, false).await;
    let req = Request::get(format!("/api/v0/projects/test/live?runs={id}"))
        .header(header::AUTHORIZATION, server.bearer())
        .body(Body::empty())
        .unwrap();
    let res = server.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let mut body = res.into_body();

    server.log(batch(other, 0, 100, 1)).await;
    server.log(batch(run, 0, 0, 3)).await;
    let finish = RunFinish {
        run_id: run.as_bytes().to_vec(),
        state: RunExitState::Finished as i32,
        ..Default::default()
    };
    server.send("/api/v0/runs/finish", finish, false).await;

    let mut text = String::new();
    while !text.contains("event: state") {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("event in time");
        if let Some(Ok(frame)) = frame
            && let Ok(data) = frame.into_data()
        {
            text.push_str(&String::from_utf8_lossy(&data));
        }
    }
    assert!(text.contains("event: points"), "{text}");
    assert!(text.contains(r#""step":[0,0,1,1,2,2]"#), "{text}");
    assert!(text.contains(r#""time_ms":[1700000000000,"#), "{text}");
    assert!(
        !text.contains(r#""step":[100,100]"#),
        "unwatched run: {text}"
    );
    let run_uuid = uuid::Uuid::from_bytes(*run.as_bytes());
    assert!(
        text.contains(&format!(r#"{{"run":"{run_uuid}","state":"finished"}}"#)),
        "{text}"
    );
    server.stop().await;
}

#[tokio::test]
async fn heartbeat_skips_runs_that_just_logged() {
    let server = server!();
    let run = RunId::new();
    let id = uuid::Uuid::from_bytes(*run.as_bytes());
    server.init(run, false).await;
    let heartbeat_at = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT (extract(epoch from heartbeat_at) * 1e6)::bigint FROM runs WHERE id = $1",
        )
        .bind(id)
        .fetch_one(server.state.pg())
        .await
        .unwrap()
    };
    let beat = Heartbeat {
        run_id: run.as_bytes().to_vec(),
        client_time_ms: 0,
    };

    let before = heartbeat_at().await;
    assert_eq!(
        server
            .send("/api/v0/runs/heartbeat", beat.clone(), false)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(heartbeat_at().await, before, "recent activity: no write");

    sqlx::query("UPDATE runs SET heartbeat_at = now() - interval '1 minute' WHERE id = $1")
        .bind(id)
        .execute(server.state.pg())
        .await
        .unwrap();
    let stale = heartbeat_at().await;
    assert_eq!(
        server.send("/api/v0/runs/heartbeat", beat, false).await.0,
        StatusCode::OK
    );
    assert!(heartbeat_at().await > stale, "stale heartbeat is refreshed");

    let unknown = Heartbeat {
        run_id: RunId::new().as_bytes().to_vec(),
        client_time_ms: 0,
    };
    assert_eq!(
        server
            .send("/api/v0/runs/heartbeat", unknown, false)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    server.stop().await;
}
