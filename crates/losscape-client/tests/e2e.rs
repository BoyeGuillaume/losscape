//! Client against a real server on fresh databases (skipped without DATABASE_URL /
//! CLICKHOUSE_URL, see deploy/docker-compose.dev.yml).

use std::path::Path;
use std::time::Duration;

use losscape_client::{FinishOutcome, Mode, Run, RunOptions, SyncStatus, sync};
use losscape_core::proto::v0::RunExitState;
use losscape_server::testing::TestServer;
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
    RunOptions::new(mode, root, "e2e")
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
    let run = Run::start(RunOptions::new(Mode::Offline, root.path(), "e2e")).unwrap();
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
    let mode = Mode::Online {
        server: format!("http://{addr}"),
        api_key: losscape_server::testing::API_KEY.into(),
    };
    let run = Run::start(RunOptions::new(mode, root.path(), "e2e")).unwrap();
    let id = run.id().to_string();
    log_steps(&run, 0, 500);
    tokio::time::sleep(Duration::from_millis(700)).await;

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let Some(server) = TestServer::start_on(listener).await else {
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
    let run = Run::start(RunOptions::new(mode, root.path(), "e2e")).unwrap();
    log_steps(&run, 0, 100);
    let (run, outcome) = finish(run, RunExitState::Finished, Duration::from_secs(10)).await;
    assert!(
        matches!(&outcome, FinishOutcome::Failed(msg) if msg.contains("unauthorized")),
        "{outcome:?}"
    );
    assert!(run.dir().exists(), "data kept for a later sync");
    server.stop().await;
}
