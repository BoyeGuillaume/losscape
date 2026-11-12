//! End-to-end tests against real Postgres + ClickHouse (see deploy/docker-compose.dev.yml).
//! Each test gets fresh databases. Skipped when DATABASE_URL / CLICKHOUSE_URL are unset.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use losscape_core::PROTOCOL_HEADER;
use losscape_core::ids::RunId;
use losscape_core::proto::v0::{
    Ack, ConsoleLine, ConsoleStream, Heartbeat, LogBatch, RunEnvironment, RunExitState, RunFinish,
    RunInit, RunInitResponse, ScalarColumns,
};
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

    // The whole log as a file: plain text, or with its colors.
    let download = |query: &str| {
        Request::get(format!("/api/v0/runs/{id}/logs.txt{query}"))
            .header(header::AUTHORIZATION, server.bearer())
            .body(Body::empty())
            .unwrap()
    };
    let (status, plain) = server.call(download("")).await;
    assert_eq!(status, StatusCode::OK);
    let plain = String::from_utf8(plain.to_vec()).unwrap();
    assert!(plain.contains("ERROR: disk almost full\n"), "{plain}");
    assert!(!plain.contains('\x1b'), "colors removed");
    assert_eq!(plain.lines().count(), 6, "every line, in order");
    let (_, colored) = server.call(download("?ansi=true")).await;
    assert!(String::from_utf8_lossy(&colored).contains("\x1b[31mERROR"));
    server.stop().await;
}

#[tokio::test]
async fn metric_ranges_names_definitions_and_renames() {
    let server = server!();
    let run = RunId::new();
    let id = uuid::Uuid::from_bytes(*run.as_bytes()).simple().to_string();
    server.init(run, false).await;
    let mut b = batch(run, 0, 0, 100);
    // A NaN must not become the minimum; system metrics have no range.
    b.scalars.as_mut().unwrap().value[1] = f64::NAN;
    server.log(b).await;
    server.log(batch(run, 1, 100, 100)).await;

    let runs = get_json(&server, "/api/v0/projects/test/runs").await.1["runs"].clone();
    let r = &runs[0];
    assert_eq!(r["metric_min"]["loss"], 1.0 / 200.0, "across batches");
    assert_eq!(r["metric_max"]["loss"], 1.0);
    assert_eq!(r["metric_max"]["acc"], 1.0 / 2.0, "NaN at step 0 skipped");
    assert_eq!(r["renames"], serde_json::json!({}));

    // Backfill gives the same ranges for runs logged before they were kept.
    sqlx::query("UPDATE runs SET metric_min = '{}', metric_max = '{}'")
        .execute(server.state.pg())
        .await
        .unwrap();
    sqlx::query("DELETE FROM project_metric_keys")
        .execute(server.state.pg())
        .await
        .unwrap();
    assert_eq!(
        losscape_server::purge::backfill_metric_index(&server.state)
            .await
            .unwrap(),
        1
    );
    let runs = get_json(&server, "/api/v0/projects/test/runs").await.1["runs"].clone();
    assert_eq!(runs[0]["metric_min"]["loss"], 1.0 / 200.0);

    let (_, keys) = get_json(&server, "/api/v0/projects/test/metric-keys").await;
    assert_eq!(keys, serde_json::json!(["acc", "loss"]));

    let send = |method: &str, uri: String, body: serde_json::Value| {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, server.bearer())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    // The board: defaults until it's set; what's set comes back; limits are enforced.
    let board_uri = "/api/v0/projects/test/board".to_owned();
    let (_, board) = get_json(&server, &board_uri).await;
    assert_eq!(
        board,
        serde_json::json!({
            "version": 0,
            "query": "",
            "sort": {"id": "created", "desc": true},
            "limit": 10,
            "baseline": null,
            "explore": null,
            "columns": null
        })
    );
    let board = serde_json::json!({
        "version": 0,
        "query": "tag:promising",
        "sort": {"id": "summary.val/acc", "desc": true},
        "limit": 20,
        "baseline": "0190f2c1-0000-7000-8000-000000000000",
        "explore": "optimizer.lr",
        "columns": ["state", "summary.val/acc"]
    });
    assert_eq!(
        server
            .call(send("PUT", board_uri.clone(), board.clone()))
            .await
            .0,
        StatusCode::OK
    );
    let (_, stored) = get_json(&server, &board_uri).await;
    assert_eq!(stored["version"], 1);
    assert_eq!(stored["query"], "tag:promising");
    assert_eq!(stored["columns"], board["columns"]);
    // Saved from what was read before that: refused, and nothing changes.
    let stale = serde_json::json!({
        "version": 0, "query": "stale", "sort": {"id": "created", "desc": true}, "limit": 5
    });
    assert_eq!(
        server.call(send("PUT", board_uri.clone(), stale)).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(get_json(&server, &board_uri).await.1, stored);
    for limit in [0, 41] {
        let bad = serde_json::json!({
            "version": 1, "query": "", "sort": {"id": "created", "desc": true}, "limit": limit
        });
        assert_eq!(
            server.call(send("PUT", board_uri.clone(), bad)).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    let bad = serde_json::json!({
        "version": 1, "query": "", "sort": {"id": "", "desc": true}, "limit": 5
    });
    assert_eq!(
        server.call(send("PUT", board_uri.clone(), bad)).await.0,
        StatusCode::BAD_REQUEST
    );

    let defs = serde_json::json!([
        {"name": "val/*", "kind": "describe", "spec": {"direction": "max", "important": true}},
        {"name": "val/pck-{k}", "kind": "family", "spec": {"direction": "max", "bounds": [0, 1]}}
    ]);
    let uri = "/api/v0/projects/test/metric-definitions".to_owned();
    let versioned = |version: i64, definitions: &serde_json::Value| serde_json::json!({"version": version, "definitions": definitions});
    assert_eq!(
        get_json(&server, &uri).await.1,
        versioned(0, &serde_json::json!([]))
    );
    assert_eq!(
        server
            .call(send("PUT", uri.clone(), versioned(0, &defs)))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        get_json(&server, &uri).await.1,
        versioned(1, &defs),
        "kept in order"
    );

    // A stale tab can't overwrite what happened since.
    let other = serde_json::json!([{"name": "a", "kind": "describe", "spec": {}}]);
    let (status, body) = server
        .call(send("PUT", uri.clone(), versioned(0, &other)))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(String::from_utf8_lossy(&body).contains("stale"));
    assert_eq!(get_json(&server, &uri).await.1, versioned(1, &defs));

    // Of two saves of one version, at the same time, exactly one passes.
    let (first, second) = tokio::join!(
        server.call(send("PUT", uri.clone(), versioned(1, &other))),
        server.call(send("PUT", uri.clone(), versioned(1, &defs)))
    );
    let mut statuses = [first.0, second.0];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT]);
    let (_, latest) = get_json(&server, &uri).await;
    assert_eq!(latest["version"], 2);
    // Back to a known state for what follows.
    assert_eq!(
        server
            .call(send("PUT", uri.clone(), versioned(2, &defs)))
            .await
            .0,
        StatusCode::OK
    );

    let twice = versioned(
        3,
        &serde_json::json!([{"name": "a", "kind": "describe", "spec": {}}, {"name": "a", "kind": "describe", "spec": {}}]),
    );
    assert_eq!(
        server.call(send("PUT", uri.clone(), twice)).await.0,
        StatusCode::BAD_REQUEST
    );
    let bad_kind = versioned(
        3,
        &serde_json::json!([{"name": "a", "kind": "merge", "spec": {}}]),
    );
    assert_eq!(
        server.call(send("PUT", uri.clone(), bad_kind)).await.0,
        StatusCode::BAD_REQUEST
    );

    // Renaming a metric of one run; names may contain slashes.
    let rename = format!("/api/v0/runs/{id}/metric-renames/val/pck-8");
    assert_eq!(
        server
            .call(send(
                "PUT",
                rename.clone(),
                serde_json::json!({"to": "val/pck-8px"})
            ))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let (_, detail) = get_json(&server, &format!("/api/v0/runs/{id}")).await;
    assert_eq!(
        (detail["renames"].clone(), detail["rename_scales"].clone()),
        (
            serde_json::json!({"val/pck-8": "val/pck-8px"}),
            serde_json::json!({})
        )
    );
    assert_eq!(
        server
            .call(send("DELETE", rename.clone(), serde_json::json!({})))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        server
            .call(send("DELETE", rename, serde_json::json!({})))
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    // Many runs at once, rescaled; runs of other projects are skipped.
    let bulk = "/api/v0/projects/test/runs/metric-renames".to_owned();
    let runs = serde_json::json!([id, RunId::new().to_string()]);
    let (status, body) = server
        .call(send(
            "POST",
            bulk.clone(),
            serde_json::json!({"runs": runs, "from": "acc", "to": "val/acc", "scale": 0.01}),
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["changed"],
        1
    );
    let (_, keys) = get_json(&server, "/api/v0/projects/test/metric-keys").await;
    assert_eq!(
        keys,
        serde_json::json!(["acc", "loss", "val/acc"]),
        "names shown by renames too"
    );
    let listed = get_json(&server, "/api/v0/projects/test/runs").await.1["runs"][0].clone();
    assert_eq!(
        (listed["renames"].clone(), listed["rename_scales"].clone()),
        (
            serde_json::json!({"acc": "val/acc"}),
            serde_json::json!({"acc": 0.01})
        )
    );
    for bad in [
        serde_json::json!({"runs": runs, "from": "acc", "to": "x", "scale": 0}),
        serde_json::json!({"runs": runs, "from": "acc", "to": "acc"}),
    ] {
        assert_eq!(
            server.call(send("POST", bulk.clone(), bad)).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    let (_, body) = server
        .call(send(
            "POST",
            bulk,
            serde_json::json!({"runs": runs, "from": "acc", "to": null}),
        ))
        .await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["changed"],
        1,
        "back to the logged name"
    );
    server.stop().await;
}

#[tokio::test]
async fn runs_list_pages_changes_and_tags() {
    let server = server!();
    let runs: Vec<RunId> = (0..3).map(|_| RunId::new()).collect();
    for run in &runs {
        server.init(*run, false).await;
    }
    let names = |page: &serde_json::Value| -> Vec<String> {
        page["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_owned())
            .collect()
    };

    // Pages, newest first, then nothing more.
    let all = listing(&server, "/api/v0/projects/test/runs").await;
    assert_eq!(all["next"], serde_json::Value::Null);
    let first = listing(&server, "/api/v0/projects/test/runs?limit=2").await;
    let next = first["next"].as_str().unwrap();
    let second = listing(
        &server,
        &format!("/api/v0/projects/test/runs?limit=2&after={next}"),
    )
    .await;
    assert_eq!(second["next"], serde_json::Value::Null);
    assert_eq!([names(&first), names(&second)].concat(), names(&all));
    assert_eq!(
        get_json(&server, "/api/v0/projects/test/runs?after=nope")
            .await
            .0,
        StatusCode::BAD_REQUEST
    );

    // Changes since a listing: none yet (past the overlap), then the runs tagged.
    let since = all["as_of_ms"].as_i64().unwrap() + 10_000;
    let changed_since = format!("/api/v0/projects/test/runs?since_ms={since}");
    assert_eq!(
        names(&listing(&server, &changed_since).await),
        Vec::<String>::new()
    );

    let send = |method: &str, uri: &str, body: serde_json::Value| {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, server.bearer())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let ids: Vec<String> = runs[..2].iter().map(|r| r.to_string()).collect();
    let (status, body) = server
        .call(send(
            "POST",
            "/api/v0/projects/test/runs/tags",
            serde_json::json!({"runs": ids, "add": ["exp/a", "baseline"], "remove": []}),
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["changed"], 2);
    let tagged = listing(&server, &changed_since).await;
    assert_eq!(tagged["runs"].as_array().unwrap().len(), 2);
    assert_eq!(
        tagged["runs"][0]["tags"],
        serde_json::json!(["baseline", "exp/a"]),
        "kept first, no duplicates"
    );

    // Removing, and nothing changes when nothing would.
    let one = vec![runs[0].to_string()];
    let (_, body) = server
        .call(send(
            "POST",
            "/api/v0/projects/test/runs/tags",
            serde_json::json!({"runs": one, "remove": ["baseline"]}),
        ))
        .await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["changed"],
        1
    );
    let (_, body) = server
        .call(send(
            "POST",
            "/api/v0/projects/test/runs/tags",
            serde_json::json!({"runs": one, "remove": ["baseline"]}),
        ))
        .await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["changed"],
        0
    );
    assert_eq!(
        server
            .call(send(
                "POST",
                "/api/v0/projects/test/runs/tags",
                serde_json::json!({"runs": one, "add": [" padded"]}),
            ))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );

    // Tag colors and descriptions; names may contain slashes.
    assert_eq!(
        server
            .call(send(
                "PUT",
                "/api/v0/projects/test/tags/exp/a",
                serde_json::json!({"color": "blue", "description": "first try"}),
            ))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        server
            .call(send(
                "PUT",
                "/api/v0/projects/test/tags/exp/a",
                serde_json::json!({"color": "#ff0000"}),
            ))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let (_, tags) = get_json(&server, "/api/v0/projects/test/tags").await;
    assert_eq!(
        tags,
        serde_json::json!([
            {"name": "baseline", "color": null, "description": "", "runs": 2},
            {"name": "exp/a", "color": "blue", "description": "first try", "runs": 2}
        ])
    );
    server.stop().await;
}

#[tokio::test]
async fn abort_reaches_the_run_through_its_heartbeat() {
    let server = server!();
    let run = RunId::new();
    server.init(run, false).await;
    let id = run.to_string();
    let beat = || Heartbeat {
        run_id: run.as_bytes().to_vec(),
        client_time_ms: 0,
    };
    let abort_asked = |body: axum::body::Bytes| Ack::decode(body).unwrap().abort;
    let (_, body) = server.send("/api/v0/runs/heartbeat", beat(), false).await;
    assert!(!abort_asked(body), "nobody asked yet");

    let abort = || {
        Request::post("/api/v0/projects/test/runs/abort")
            .header(header::AUTHORIZATION, server.bearer())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::json!({"runs": [id]}).to_string()))
            .unwrap()
    };
    let changed =
        |body: &[u8]| serde_json::from_slice::<serde_json::Value>(body).unwrap()["changed"].clone();
    let (status, body) = server.call(abort()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(changed(&body), 1);
    let (_, body) = server.call(abort()).await;
    assert_eq!(changed(&body), 0, "already asked");

    let (_, body) = server.send("/api/v0/runs/heartbeat", beat(), false).await;
    assert!(abort_asked(body), "the SDK learns about it");
    let listed = get_json(&server, "/api/v0/projects/test/runs").await.1["runs"][0].clone();
    assert!(listed["abort_requested_at_ms"].as_i64().is_some());

    // Resuming starts over; a finished run can't be aborted.
    server.init(run, true).await;
    let (_, body) = server.send("/api/v0/runs/heartbeat", beat(), false).await;
    assert!(!abort_asked(body), "resuming clears the request");
    server
        .send(
            "/api/v0/runs/finish",
            RunFinish {
                run_id: run.as_bytes().to_vec(),
                state: RunExitState::Killed as i32,
                exit_code: 130,
                ..Default::default()
            },
            false,
        )
        .await;
    let (_, body) = server.call(abort()).await;
    assert_eq!(changed(&body), 0, "not running");
    server.stop().await;
}

#[tokio::test]
async fn model_descriptions_replace_by_name_and_stay_out_of_run_lists() {
    let server = server!();
    let run = RunId::new();
    server.init(run, false).await;
    let id = run.to_string();
    let uri = format!("/api/v0/runs/{id}/models");
    assert_eq!(get_json(&server, &uri).await.1, serde_json::json!({}));

    let models = |seq, json: &str| LogBatch {
        run_id: run.as_bytes().to_vec(),
        seq,
        models_json: json.to_owned(),
        ..Default::default()
    };
    server
        .log(models(
            0,
            r#"{"model": {"params": 1}, "head": {"params": 2}}"#,
        ))
        .await;
    server.log(models(1, r#"{"model": {"params": 3}}"#)).await;
    let (status, stored) = get_json(&server, &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        stored,
        serde_json::json!({"model": {"params": 3}, "head": {"params": 2}}),
        "the latest of each name"
    );

    let (status, _) = server.send("/api/v0/ingest", models(2, "[1]"), false).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let listed = listing(&server, "/api/v0/projects/test/runs").await;
    assert!(listed["runs"][0].get("models").is_none());
    let (_, detail) = get_json(&server, &format!("/api/v0/runs/{id}")).await;
    assert!(detail.get("models").is_none());
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
    // For the sign-in and About pages, so without signing in; `get` sends no credentials.
    let (status, body) = server.call(get("/api/v0/app")).await;
    assert_eq!(status, StatusCode::OK);
    let app: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(app["name"], "losscape");
    assert_eq!(app["protocol"], 0);
    assert!(app["favicon"].is_null() && app["auth_background"].is_null());
    assert_eq!(
        server.call(get("/branding/favicon")).await.0,
        StatusCode::NOT_FOUND
    );
    // Endpoints that don't exist say so, in the format of the others.
    for uri in ["/api/v0/nope", "/api/v0/nope/further", "/api/v9/runs"] {
        let (status, body) = server.call(get(uri)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], "unknown_endpoint");
    }
    server.stop().await;
}

async fn listing(server: &TestServer, uri: &str) -> serde_json::Value {
    let (status, body) = get_json(server, uri).await;
    assert_eq!(status, StatusCode::OK, "{uri}");
    body
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

    let runs = get_json(&server, "/api/v0/projects/test/runs").await.1["runs"].clone();
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
    let runs = get_json(&server, "/api/v0/projects/test/runs").await.1["runs"].clone();
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
    let runs = get_json(&server, "/api/v0/projects/test/runs").await.1["runs"].clone();
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

fn console_line(line: u64, text: &str) -> ConsoleLine {
    ConsoleLine {
        line,
        time_ms: 1,
        stream: ConsoleStream::Stdout as i32,
        text: text.into(),
    }
}

#[tokio::test]
async fn nul_in_a_batch_is_dropped_and_harms_nobody() {
    let server = server!();
    let run = RunId::new();
    server.init(run, false).await;
    let batch = LogBatch {
        run_id: run.as_bytes().to_vec(),
        seq: 0,
        summary_json: r#"{"a\u0000b": "c\u0000d"}"#.into(),
        config_json: r#"{"e": ["f\u0000"]}"#.into(),
        scalars: Some(ScalarColumns {
            keys: vec!["lo\0ss".into()],
            key_idx: vec![0],
            step: vec![0],
            time_ms: vec![0],
            value: vec![1.0],
        }),
        ..Default::default()
    };
    let (status, _) = server.send("/api/v0/ingest", batch, false).await;
    assert_eq!(status, StatusCode::OK);
    let id = uuid::Uuid::from_bytes(*run.as_bytes());
    let (summary, config): (serde_json::Value, serde_json::Value) =
        sqlx::query_as("SELECT summary, config FROM runs WHERE id = $1")
            .bind(id)
            .fetch_one(server.state.pg())
            .await
            .unwrap();
    assert_eq!(summary["ab"], "cd");
    assert_eq!(config["e"][0], "f");
    let keys: Vec<String> = sqlx::query_scalar("SELECT key FROM project_metric_keys")
        .fetch_all(server.state.pg())
        .await
        .unwrap();
    assert_eq!(keys, ["loss"]);

    let named = RunId::new();
    let init = RunInit {
        run_id: named.as_bytes().to_vec(),
        project: "test".into(),
        name: "na\0me".into(),
        notes: "no\0tes".into(),
        group: "gr\0oup".into(),
        tags: vec!["t\0ag".into()],
        config_json: "{}".into(),
        ..Default::default()
    };
    let (status, _) = server.send("/api/v0/runs/init", init, false).await;
    assert_eq!(status, StatusCode::OK);
    server.stop().await;
}

#[tokio::test]
async fn a_run_the_database_refuses_holds_up_only_itself() {
    let server = server!();
    let (poisoned, healthy) = (RunId::new(), RunId::new());
    server.init(poisoned, false).await;
    server.init(healthy, false).await;
    let refuse = format!(
        "CREATE FUNCTION refuse() RETURNS trigger AS $$ BEGIN
           IF NEW.id = '{}' THEN RAISE EXCEPTION 'refused'; END IF; RETURN NEW;
         END $$ LANGUAGE plpgsql",
        uuid::Uuid::from_bytes(*poisoned.as_bytes())
    );
    sqlx::query(&refuse)
        .execute(server.state.pg())
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER refuse BEFORE UPDATE ON runs FOR EACH ROW EXECUTE FUNCTION refuse()",
    )
    .execute(server.state.pg())
    .await
    .unwrap();
    let batch = |run: RunId, seq: u64| LogBatch {
        run_id: run.as_bytes().to_vec(),
        seq,
        ..Default::default()
    };
    // Sent together, to share a flush, repeatedly in case they didn't.
    for seq in 0..5 {
        let (bad, good) = tokio::join!(
            server.send("/api/v0/ingest", batch(poisoned, seq), false),
            server.send("/api/v0/ingest", batch(healthy, seq), false),
        );
        assert_eq!(bad.0, StatusCode::BAD_REQUEST);
        assert_eq!(good.0, StatusCode::OK);
    }
    server.stop().await;
}

#[tokio::test]
async fn bodies_that_would_decode_to_too_much_are_refused() {
    let server = server!();
    // Well under 1 MB compressed. Decoded, these empty console lines would take gigabytes.
    let bomb = [0x3au8, 0x00].repeat(32 * 1024 * 1024);
    let compressed = zstd::encode_all(&bomb[..], 3).unwrap();
    assert!(compressed.len() < 1024 * 1024);
    let req = Request::post("/api/v0/ingest")
        .header(header::AUTHORIZATION, server.bearer())
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(header::CONTENT_ENCODING, "zstd")
        .header(PROTOCOL_HEADER, "0")
        .body(Body::from(compressed))
        .unwrap();
    let (status, body) = server.call(req).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(error_of(&body).starts_with("payload_too_large"));
    server.stop().await;
}

#[tokio::test]
async fn series_requests_are_limited_and_streamed() {
    let server = server!();
    let run = RunId::new();
    server.init(run, false).await;
    let batch = LogBatch {
        run_id: run.as_bytes().to_vec(),
        seq: 0,
        scalars: Some(ScalarColumns {
            keys: vec!["m".into()],
            key_idx: vec![0; 3],
            step: vec![0, 1, 2],
            time_ms: vec![0; 3],
            value: vec![1.0, 2.0, 3.0],
        }),
        ..Default::default()
    };
    server.send("/api/v0/ingest", batch, false).await;
    let id = uuid::Uuid::from_bytes(*run.as_bytes()).simple().to_string();
    let series = |ranges: usize, to_tile: u64| {
        let range = serde_json::json!({"run": id, "key": "m", "level": 0, "from_tile": 0, "to_tile": to_tile});
        Request::post("/api/v0/series?format=json")
            .header(header::AUTHORIZATION, server.bearer())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "ranges": vec![range; ranges] }).to_string(),
            ))
            .unwrap()
    };
    let (status, body) = server.call(series(1, 1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8_lossy(&body).lines().count(), 3);
    // Duplicates add up: each is as much work as the first.
    assert_eq!(server.call(series(32, 64)).await.0, StatusCode::OK);
    assert_eq!(server.call(series(33, 64)).await.0, StatusCode::BAD_REQUEST);
    server.stop().await;
}

#[tokio::test]
async fn the_log_download_ends_whatever_the_line_numbers() {
    let server = server!();
    let run = RunId::new();
    server.init(run, false).await;
    let batch = LogBatch {
        run_id: run.as_bytes().to_vec(),
        seq: 0,
        console: vec![
            console_line(0, "first"),
            console_line(1 << 63, "far away"),
            console_line(u64::MAX, "the very end"),
        ],
        ..Default::default()
    };
    let (status, _) = server.send("/api/v0/ingest", batch, false).await;
    assert_eq!(status, StatusCode::OK);
    let id = uuid::Uuid::from_bytes(*run.as_bytes()).simple().to_string();
    let req = Request::get(format!("/api/v0/runs/{id}/logs.txt"))
        .header(header::AUTHORIZATION, server.bearer())
        .body(Body::empty())
        .unwrap();
    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(10), server.call(req))
        .await
        .expect("finished");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        String::from_utf8_lossy(&body),
        "first\nfar away\nthe very end\n"
    );
    server.stop().await;
}

#[tokio::test]
async fn live_streams_end_when_access_does() {
    let server = server!();
    let (_, viewer_key) = common::user(&server.state, "viewer").await;
    let run = RunId::new();
    server.init(run, false).await;
    let project: i64 = sqlx::query_scalar("SELECT id FROM projects WHERE name = 'test'")
        .fetch_one(server.state.pg())
        .await
        .unwrap();
    let viewer: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'viewer'")
        .fetch_one(server.state.pg())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO project_members (project_id, user_id, role) VALUES ($1, $2, 'viewer')",
    )
    .bind(project)
    .bind(viewer)
    .execute(server.state.pg())
    .await
    .unwrap();
    let id = uuid::Uuid::from_bytes(*run.as_bytes()).simple().to_string();
    let req = Request::get(format!("/api/v0/projects/test/live?runs={id}"))
        .header(header::AUTHORIZATION, format!("Bearer {viewer_key}"))
        .body(Body::empty())
        .unwrap();
    let res = server.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let mut body = res.into_body();

    // Still a member after several checks: it stays open.
    let quiet = tokio::time::timeout(std::time::Duration::from_millis(700), body.frame()).await;
    assert!(quiet.is_err(), "the stream ended while still allowed");

    sqlx::query("DELETE FROM project_members WHERE user_id = $1")
        .bind(viewer)
        .execute(server.state.pg())
        .await
        .unwrap();
    server.state.auth_caches().forget_project(project);
    let ended = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(frame) = body.frame().await {
            frame.unwrap();
        }
    })
    .await;
    assert!(ended.is_ok(), "the stream outlived the membership");
    server.stop().await;
}

#[tokio::test]
async fn a_tag_given_twice_is_one_tag() {
    let server = server!();
    server.create_project("test").await;
    let run = RunId::new();
    let init = RunInit {
        run_id: run.as_bytes().to_vec(),
        project: "test".into(),
        tags: vec!["a".into(), "b".into(), "a".into()],
        config_json: "{}".into(),
        ..Default::default()
    };
    assert_eq!(
        server.send("/api/v0/runs/init", init, false).await.0,
        StatusCode::OK
    );
    let tags: Vec<String> = sqlx::query_scalar("SELECT tags FROM runs WHERE id = $1")
        .bind(uuid::Uuid::from_bytes(*run.as_bytes()))
        .fetch_one(server.state.pg())
        .await
        .unwrap();
    assert_eq!(tags, ["a", "b"]);
    server.stop().await;
}

#[tokio::test]
async fn history_streams_every_point_as_csv() {
    let Some(server) = TestServer::start().await else {
        return;
    };
    let run = RunId::new();
    server.init(run, false).await;
    server.log(batch(run, 0, 0, 5)).await;
    let id = uuid::Uuid::from_bytes(*run.as_bytes());
    let get = |query: String| {
        Request::get(format!("/api/v0/projects/test/history?{query}"))
            .header(header::AUTHORIZATION, server.bearer())
            .body(Body::empty())
            .unwrap()
    };

    let (status, csv) = server
        .call(get(format!("run={id}&key=loss&key=acc&key=missing")))
        .await;
    assert_eq!(status, StatusCode::OK, "{}", error_of(&csv));
    let csv = String::from_utf8(csv.to_vec()).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(
        lines[0],
        r#""run","run_id","metric","step","time_utc","value""#
    );
    assert_eq!(
        lines.len(),
        1 + 2 * 5,
        "every point of both metrics:\n{csv}"
    );
    assert_eq!(
        lines[1],
        format!(r#""brave-falcon-1","{id}","acc",0,"2023-11-14 22:13:20.000",1"#),
        "by metric, then step"
    );
    assert!(lines[10].starts_with(&format!(r#""brave-falcon-1","{id}","loss",4,"#)));

    let (_, some) = server
        .call(get(format!("run={id}&key=loss&min_step=1&max_step=2")))
        .await;
    let some = String::from_utf8(some.to_vec()).unwrap();
    let steps: Vec<&str> = some
        .lines()
        .skip(1)
        .map(|l| l.split(',').nth(3).unwrap())
        .collect();
    assert_eq!(steps, ["1", "2"], "{some}");

    // Asking is checked: names, run ids, and the project they're in.
    let (status, body) = server.call(get(format!("run={id}"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{}", error_of(&body));
    let (status, _) = server.call(get("key=loss".into())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = server
        .call(get(format!("run={}&key=loss", uuid::Uuid::now_v7())))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a run of no project");
    assert_eq!(server.create_project("other").await, StatusCode::CREATED);
    let (status, _) = server
        .call(
            Request::get(format!("/api/v0/projects/other/history?run={id}&key=loss"))
                .header(header::AUTHORIZATION, server.bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a run of another project");
    server.stop().await;
}
