//! Client against a real server on fresh databases (skipped without DATABASE_URL /
//! CLICKHOUSE_URL, see deploy/docker-compose.dev.yml).

use std::path::Path;
use std::time::Duration;

use losscape_client::{FinishOutcome, Mode, Run, RunOptions, SyncStatus, sync};
use losscape_core::proto::v1::RunExitState;
use losscape_server::testing::{PROJECT, TestServer};
use serde_json::json;

macro_rules! server {
    () => {
        match TestServer::start().await {
            Some(server) => server,
            None => return,
        }
    };
}

fn online(server: &TestServer, root: &Path) -> RunOptions {
    let mode = Mode::Online {
        server: server.url.clone(),
        api_key: server.api_key.clone(),
    };
    RunOptions::new(mode, root, PROJECT)
}

/// Logs `steps` steps of 4 metrics starting at `first`.
fn log_steps(run: &Run, first: u64, steps: u64) {
    for step in first..first + steps {
        let s = step as f64;
        run.log(
            step,
            &[
                ("loss", 1.0 / (s + 1.0)),
                ("acc", s / 1e4),
                ("lr", 1e-3),
                ("epoch", (step / 100) as f64),
            ],
        );
    }
}

/// `Run::finish` blocks; keep it off the runtime's worker threads.
async fn finish(run: Run, state: RunExitState, timeout: Duration) -> (Run, FinishOutcome) {
    tokio::task::spawn_blocking(move || {
        let outcome = run.finish(state, 0, timeout);
        (run, outcome)
    })
    .await
    .unwrap()
}

async fn sync_root(root: &Path, server: &TestServer) -> Vec<SyncStatus> {
    let (root, url, key) = (root.to_owned(), server.url.clone(), server.api_key.clone());
    std::thread::spawn(move || sync(&root, &url, &key).unwrap())
        .join()
        .unwrap()
        .into_iter()
        .map(|r| r.status)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn online_run_uploads_everything_and_cleans_up() {
    let server = server!();
    let root = tempfile::tempdir().unwrap();
    let mut opts = online(&server, root.path());
    opts.name = "brave-falcon-1".into();
    opts.config = json!({"lr": 1e-3}).as_object().unwrap().clone();
    let run = Run::start(opts).unwrap();
    let id = run.id().to_string();

    log_steps(&run, 0, 5_000);
    run.update_summary(json!({"best_acc": 0.42}).as_object().unwrap().clone());
    let (run, outcome) = finish(run, RunExitState::Finished, Duration::from_secs(30)).await;

    assert_eq!(outcome, FinishOutcome::Uploaded);
    assert!(!run.dir().exists(), "local log deleted after upload");
    assert_eq!(server.points(&id).await, (20_000, 20_000));
    let (state, code, summary) = server.run(&id).await.unwrap();
    assert_eq!((state.as_str(), code), ("finished", Some(0)));
    assert_eq!(
        summary["loss"],
        json!(1.0 / 5000.0),
        "summary holds the last logged value"
    );
    assert_eq!(summary["best_acc"], json!(0.42));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_run_is_uploaded_by_sync() {
    let server = server!();
    let root = tempfile::tempdir().unwrap();
    let run = Run::start(RunOptions::new(Mode::Offline, root.path(), PROJECT)).unwrap();
    let id = run.id().to_string();
    log_steps(&run, 0, 1_000);
    let (run, outcome) = finish(run, RunExitState::Finished, Duration::from_secs(5)).await;
    assert!(
        matches!(outcome, FinishOutcome::SavedLocally { reason, .. } if reason == "offline mode")
    );
    assert!(run.dir().exists());
    assert!(
        server.run(&id).await.is_none(),
        "nothing sent in offline mode"
    );

    assert_eq!(
        sync_root(root.path(), &server).await,
        [SyncStatus::Uploaded]
    );
    assert!(!run.dir().exists());
    assert_eq!(server.points(&id).await, (4_000, 4_000));
    assert_eq!(server.run(&id).await.unwrap().0, "finished");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_down_at_start_catches_up() {
    // Reserve a port, release it, start the run against it while nothing listens.
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    // The key is made up front and registered when the server starts.
    let api_key = losscape_server::auth::secrets::new_api_key().secret;
    let mode = Mode::Online {
        server: format!("http://{addr}"),
        api_key: api_key.clone(),
    };
    let run = Run::start(RunOptions::new(mode, root.path(), PROJECT)).unwrap();
    let id = run.id().to_string();
    log_steps(&run, 0, 500);
    tokio::time::sleep(Duration::from_millis(700)).await;

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let Some(server) =
        TestServer::start_with(listener, |_| Default::default(), Some(api_key)).await
    else {
        return;
    };
    log_steps(&run, 500, 500);
    let (_, outcome) = finish(run, RunExitState::Finished, Duration::from_secs(30)).await;
    assert_eq!(outcome, FinishOutcome::Uploaded);
    assert_eq!(server.points(&id).await, (4_000, 4_000));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_continues_steps_and_batches() {
    let server = server!();
    let root = tempfile::tempdir().unwrap();
    let run = Run::start(online(&server, root.path())).unwrap();
    let id = run.id();
    log_steps(&run, 0, 300);
    let (_, outcome) = finish(run, RunExitState::Finished, Duration::from_secs(30)).await;
    assert_eq!(outcome, FinishOutcome::Uploaded);

    let mut opts = online(&server, root.path());
    opts.run_id = Some(id);
    opts.resume = true;
    let run = tokio::task::spawn_blocking(move || Run::start(opts).unwrap())
        .await
        .unwrap();
    assert_eq!(run.next_step(), Some(300));
    log_steps(&run, 300, 300);
    let (_, outcome) = finish(run, RunExitState::Finished, Duration::from_secs(30)).await;
    assert_eq!(outcome, FinishOutcome::Uploaded);
    assert_eq!(
        server.points(&id.to_string()).await,
        (2_400, 2_400),
        "no batch overwritten"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropped_run_is_kept_and_synced_as_killed() {
    let server = server!();
    let root = tempfile::tempdir().unwrap();
    let run = Run::start(online(&server, root.path())).unwrap();
    let id = run.id().to_string();
    let dir = run.dir().to_owned();
    log_steps(&run, 0, 2_000);
    // Dropped without finish(): flushed to the local log, upload abandoned.
    tokio::task::spawn_blocking(move || drop(run))
        .await
        .unwrap();
    assert!(dir.exists());

    assert_eq!(
        sync_root(root.path(), &server).await,
        [SyncStatus::Uploaded]
    );
    assert_eq!(server.points(&id).await.1, 8_000);
    assert_eq!(server.run(&id).await.unwrap().0, "killed");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejected_api_key_keeps_data_locally() {
    let server = server!();
    let root = tempfile::tempdir().unwrap();
    let mode = Mode::Online {
        server: server.url.clone(),
        api_key: "wrong".into(),
    };
    let run = Run::start(RunOptions::new(mode, root.path(), PROJECT)).unwrap();
    log_steps(&run, 0, 100);
    let (run, outcome) = finish(run, RunExitState::Finished, Duration::from_secs(10)).await;
    assert!(
        matches!(&outcome, FinishOutcome::Failed(msg) if msg.contains("unauthorized")),
        "{outcome:?}"
    );
    assert!(run.dir().exists(), "data kept for a later sync");
    server.stop().await;
}

/// Bytes zstd can't compress (xorshift).
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len.div_ceil(8))
        .flat_map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x.to_le_bytes()
        })
        .take(len)
        .collect()
}

/// A checkpoint folder: weights over two parts (incompressible), a config (compressible).
fn checkpoint_folder(dir: &Path, seed: u64) -> (Vec<u8>, Vec<u8>) {
    let weights = noise(losscape_core::CHECKPOINT_PART_SIZE as usize + 12_345, seed);
    let config = format!("{{\"seed\": {seed}}}\n").repeat(500).into_bytes();
    std::fs::create_dir_all(dir.join("extra")).unwrap();
    std::fs::write(dir.join("model.bin"), &weights).unwrap();
    std::fs::write(dir.join("extra/config.json"), &config).unwrap();
    (weights, config)
}

async fn api(server: &TestServer, path: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(format!("{}/api/v1/{path}", server.url))
        .bearer_auth(&server.api_key)
        .send()
        .await
        .unwrap()
}

/// The run's committed checkpoints, by step.
async fn checkpoints(server: &TestServer, run_id: &str) -> Vec<serde_json::Value> {
    let listed: serde_json::Value = api(server, &format!("runs/{run_id}/checkpoints"))
        .await
        .bytes()
        .await
        .map(|b| serde_json::from_slice(&b).unwrap())
        .unwrap();
    listed.as_array().unwrap().clone()
}

/// A checkpoint's file, by path, as downloaded.
async fn checkpoint_file(
    server: &TestServer,
    checkpoint: &serde_json::Value,
    path: &str,
) -> Vec<u8> {
    let id = checkpoint["id"].as_str().unwrap();
    let detail: serde_json::Value = api(server, &format!("checkpoints/{id}"))
        .await
        .bytes()
        .await
        .map(|b| serde_json::from_slice(&b).unwrap())
        .unwrap();
    let file = detail["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == path)
        .unwrap_or_else(|| panic!("{path} in {detail}"));
    let response = api(server, &format!("checkpoints/{id}/files/{}", file["index"])).await;
    assert!(response.status().is_success());
    response.bytes().await.unwrap().to_vec()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checkpoints_upload_with_the_run() {
    let server = server!();
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("ckpt");
    let run = Run::start(online(&server, root.path())).unwrap();
    let id = run.id().to_string();

    log_steps(&run, 0, 100);
    let (weights_10, config_10) = checkpoint_folder(&src, 10);
    run.log_checkpoint(&src, 10, false).unwrap();
    // Saved again to the same place: what was logged at step 10 doesn't change.
    let (weights_20, _) = checkpoint_folder(&src, 20);
    run.log_checkpoint(&src, 20, true).unwrap();
    assert!(!src.exists(), "moved");
    assert!(
        run.log_checkpoint(&src, 30, false).is_err(),
        "nothing there"
    );
    let (run, outcome) = finish(run, RunExitState::Finished, Duration::from_secs(30)).await;

    assert_eq!(outcome, FinishOutcome::Uploaded);
    assert!(!run.dir().exists(), "local copies deleted after upload");
    assert!(
        run.log_checkpoint(root.path(), 40, false).is_err(),
        "finished"
    );
    let listed = checkpoints(&server, &id).await;
    let steps: Vec<_> = listed.iter().map(|c| c["step"].as_u64().unwrap()).collect();
    assert_eq!(steps, [10, 20]);
    assert_eq!(listed[0]["file_count"], 2);
    assert!(
        listed[0]["stored_size"].as_u64() < listed[0]["size"].as_u64(),
        "the config was compressed"
    );
    assert_eq!(
        checkpoint_file(&server, &listed[0], "model.bin").await,
        weights_10
    );
    assert_eq!(
        checkpoint_file(&server, &listed[0], "extra/config.json").await,
        config_10
    );
    assert_eq!(
        checkpoint_file(&server, &listed[1], "model.bin").await,
        weights_20
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_checkpoints_are_uploaded_by_sync() {
    let server = server!();
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("ckpt");
    let run = Run::start(RunOptions::new(Mode::Offline, root.path(), PROJECT)).unwrap();
    let id = run.id().to_string();
    log_steps(&run, 0, 10);
    let (weights, _) = checkpoint_folder(&src, 1);
    run.log_checkpoint(&src, 9, false).unwrap();
    assert_eq!(run.local_checkpoints().unwrap()[0].0, 9);
    let (run, _) = finish(run, RunExitState::Finished, Duration::from_secs(5)).await;
    assert!(run.dir().exists());

    assert_eq!(
        sync_root(root.path(), &server).await,
        [SyncStatus::Uploaded]
    );
    assert!(!run.dir().exists());
    let listed = checkpoints(&server, &id).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(
        checkpoint_file(&server, &listed[0], "model.bin").await,
        weights
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checkpoint_upload_cut_short_resumes_with_sync() {
    let server = server!();
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("ckpt");
    let run = Run::start(online(&server, root.path())).unwrap();
    let id = run.id().to_string();
    let dir = run.dir().to_owned();
    log_steps(&run, 0, 10);
    let (weights, _) = checkpoint_folder(&src, 3);
    run.log_checkpoint(&src, 5, false).unwrap();
    // Stopped as if killed, before its upload could end (uploads are fast).
    tokio::task::spawn_blocking(move || drop(run))
        .await
        .unwrap();
    assert!(dir.join("checkpoints/5").exists());

    assert_eq!(
        sync_root(root.path(), &server).await,
        [SyncStatus::Uploaded]
    );
    let listed = checkpoints(&server, &id).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(
        checkpoint_file(&server, &listed[0], "model.bin").await,
        weights
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refused_checkpoints_stay_locally() {
    let server = server!();
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("ckpt");
    checkpoint_folder(&src, 1);
    let run = Run::start(online(&server, root.path())).unwrap();
    let id = run.id();
    run.log_checkpoint(&src, 5, false).unwrap();
    let (_, outcome) = finish(run, RunExitState::Finished, Duration::from_secs(30)).await;
    assert_eq!(outcome, FinishOutcome::Uploaded);

    // Resumed, the same step again: the server keeps the first one.
    let mut opts = online(&server, root.path());
    opts.run_id = Some(id);
    opts.resume = true;
    let run = tokio::task::spawn_blocking(move || Run::start(opts).unwrap())
        .await
        .unwrap();
    run.log_checkpoint(&src, 5, false).unwrap();
    let (run, outcome) = finish(run, RunExitState::Finished, Duration::from_secs(30)).await;
    assert!(
        matches!(&outcome, FinishOutcome::SavedLocally { pending_batches: 0, reason }
            if reason.contains("step 5 refused")),
        "{outcome:?}"
    );
    assert_eq!(run.local_checkpoints().unwrap().len(), 1, "kept");
    let synced = sync_root(root.path(), &server).await;
    assert!(
        matches!(&synced[..], [SyncStatus::Failed(msg)] if msg.contains("refused")),
        "not sent again: {synced:?}"
    );
    assert_eq!(checkpoints(&server, &id.to_string()).await.len(), 1);
    server.stop().await;
}
