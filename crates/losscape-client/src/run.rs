//! A live run: `log()` appends to a buffer; a background thread turns it into batches in
//! the local log every `batch_interval` and uploads them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use losscape_core::ids::RunId;
use losscape_core::proto::v0::{Ack, LogBatch, RunExitState, RunFinish, RunInit, RunInitResponse};
use prost::Message;
use serde_json::{Map, Value};
use tokio::sync::Notify;

use crate::buffer::Buffer;
use crate::console::{Console, MAX_LINES_PER_BATCH};
use crate::env::Extra;
use crate::system::Sampler;
use crate::upload::{Client, Live, UploadError, upload_run};
use crate::wal::{self, FINISH_FILE, INIT_FILE, RunLock, WalReader, WalWriter};

/// Buffered points that trigger a batch before the interval is up.
pub const WAKE_ROWS: usize = 50_000;
/// Also how soon a run learns it was aborted.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(5);
const RESUME_TIMEOUT: Duration = Duration::from_secs(15);
const ZSTD_LEVEL: i32 = 3;
/// How long finish waits for console output still in flight.
const CONSOLE_DRAIN: Duration = Duration::from_millis(500);
/// Metrics under this prefix are system metrics (step = unix seconds).
pub const SYSTEM_PREFIX: &str = "system/";

#[derive(Clone, Debug)]
pub enum Mode {
    Online {
        server: String,
        api_key: String,
    },
    /// Only the local log; upload later with `sync`.
    Offline,
}

#[derive(Clone, Debug)]
pub struct RunOptions {
    pub mode: Mode,
    /// Local logs go to `<root>/runs/<run id>/`.
    pub root: PathBuf,
    pub run_id: Option<RunId>,
    /// Continue an existing run: steps and batch numbers pick up where it stopped.
    pub resume: bool,
    pub project: String,
    pub name: String,
    pub group: String,
    pub tags: Vec<String>,
    pub notes: String,
    pub config: Map<String, Value>,
    pub python_version: String,
    pub sdk_version: String,
    pub batch_interval: Duration,
    /// Capture stdout and stderr (Unix).
    pub console: bool,
    /// Sample system metrics this often; None: off.
    pub system_interval: Option<Duration>,
    pub env: Extra,
}

impl RunOptions {
    pub fn new(mode: Mode, root: impl Into<PathBuf>, project: impl Into<String>) -> Self {
        Self {
            mode,
            root: root.into(),
            run_id: None,
            resume: false,
            project: project.into(),
            name: String::new(),
            group: String::new(),
            tags: Vec::new(),
            notes: String::new(),
            config: Map::new(),
            python_version: String::new(),
            sdk_version: losscape_core::VERSION.to_owned(),
            batch_interval: Duration::from_millis(100),
            console: false,
            system_interval: None,
            env: Extra::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("run {0} is already open in another process")]
    InUse(RunId),
    #[error("local log {}: {err}", dir.display())]
    Local { dir: PathBuf, err: io::Error },
    #[error("resuming run {run_id}: {err}")]
    Resume { run_id: RunId, err: UploadError },
    #[error("resuming run {0} needs the server, or its local log on this machine")]
    ResumeOffline(RunId),
}

#[derive(Clone, Debug, PartialEq)]
pub enum FinishOutcome {
    /// Everything is on the server; the local log was deleted.
    Uploaded,
    /// Kept in the local log (offline mode, or the server wasn't reachable in time).
    SavedLocally {
        pending_batches: u64,
        reason: String,
    },
    /// The server rejected the run, or the local log couldn't be written.
    Failed(String),
}

pub struct Run {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
    #[cfg(unix)]
    capture: Mutex<Option<crate::console::Capture>>,
    sampler: Mutex<Option<Sampler>>,
    owner_pid: u32,
    run_id: RunId,
    dir: PathBuf,
    next_step: Option<u64>,
}

struct Shared {
    buffer: Mutex<Buffer>,
    console: Arc<Mutex<Console>>,
    /// Make a batch now (buffer is large, or finish was requested).
    wake: Notify,
    finish: Mutex<Option<RunFinish>>,
    /// Stop uploading (finish timed out).
    cancel: Notify,
    outcome: Mutex<Option<FinishOutcome>>,
    done: Condvar,
    /// Batches written to the local log so far (= next seq).
    written: AtomicU64,
    live: Live,
    /// Someone asked for this run to stop (from the server's answer to a heartbeat).
    abort: AtomicBool,
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl Run {
    /// Returns immediately for a new run: the server is contacted in the background, and
    /// nothing is lost if it can't be reached. Resuming without local state asks the
    /// server where the run stopped first.
    pub fn start(mut opts: RunOptions) -> Result<Self, StartError> {
        crate::redact::redact_json(&mut opts.config);
        for text in [&mut opts.name, &mut opts.group, &mut opts.notes] {
            crate::redact::scrub_in_place(text);
        }
        opts.tags.iter_mut().for_each(crate::redact::scrub_in_place);
        let run_id = opts.run_id.unwrap_or_default();
        let dir = opts.root.join("runs").join(run_id.to_string());
        let local = |err| StartError::Local {
            dir: dir.clone(),
            err,
        };
        wal::create_private_dir(&dir).map_err(local)?;
        let lock = wal::try_lock(&dir)
            .map_err(local)?
            .ok_or(StartError::InUse(run_id))?;
        let mut writer = WalWriter::open(&dir).map_err(local)?;
        let has_local_log = dir.join(INIT_FILE).exists();

        let init = RunInit {
            run_id: run_id.as_bytes().to_vec(),
            project: opts.project.clone(),
            name: opts.name.clone(),
            group: opts.group.clone(),
            tags: opts.tags.clone(),
            notes: opts.notes.clone(),
            config_json: if opts.config.is_empty() {
                String::new()
            } else {
                Value::Object(opts.config.clone()).to_string()
            },
            client_time_ms: now_ms(),
            resume: opts.resume,
            environment: None,
        };

        let mut next_step = None;
        let mut first_console_line = 0;
        if opts.resume {
            let logged = if has_local_log {
                logged_so_far(&dir)
            } else {
                Logged::default()
            };
            let local_next = logged.max_step.map(|s| s + 1);
            first_console_line = logged.next_console_line;
            match &opts.mode {
                Mode::Online { server, api_key } => {
                    let server = fetch_resume_point(server, api_key, &init)
                        .map_err(|err| StartError::Resume { run_id, err })?;
                    writer.start_at(server.next_seq).map_err(local)?;
                    next_step = local_next.max(server.last_step.map(|s| s + 1));
                    first_console_line = first_console_line.max(server.next_console_line);
                }
                Mode::Offline if has_local_log => next_step = local_next,
                Mode::Offline => return Err(StartError::ResumeOffline(run_id)),
            }
        }

        let client = match &opts.mode {
            Mode::Online { server, api_key } => Some(
                Client::new(server, api_key, None)
                    .map_err(|e| StartError::Resume { run_id, err: e })?,
            ),
            Mode::Offline => None,
        };
        let acked = wal::read_acked(&dir).map_err(local)?;
        let shared = Arc::new(Shared {
            buffer: Mutex::default(),
            console: Arc::new(Mutex::new(Console::new(first_console_line))),
            wake: Notify::new(),
            finish: Mutex::default(),
            cancel: Notify::new(),
            outcome: Mutex::default(),
            done: Condvar::new(),
            written: AtomicU64::new(writer.next_seq()),
            abort: AtomicBool::new(false),
            live: Live {
                acked: Arc::new(AtomicU64::new(acked)),
                ..Live::default()
            },
        });
        if !opts.config.is_empty() {
            // Also sent in the first batch, so a resumed run merges it into its config.
            shared
                .buffer
                .lock()
                .expect("not poisoned")
                .update_config(opts.config.clone());
        }

        // The console is captured before the environment is sent: it says whether capture
        // is on, and why not (the SDK may have put a reason in `console.off`).
        let mut env = opts.env;
        let mut console_state = match env.details.remove("console") {
            Some(Value::Object(state)) => state,
            _ => Map::new(),
        };
        #[cfg(unix)]
        let capture = if opts.console {
            match crate::console::Capture::start(shared.console.clone()) {
                Ok(capture) => Some(capture),
                Err(err) => {
                    eprintln!("losscape: console capture is off: {err}");
                    console_state.insert("error".into(), Value::String(err.to_string()));
                    None
                }
            }
        } else {
            None
        };
        #[cfg(unix)]
        let captured = capture.is_some();
        #[cfg(not(unix))]
        let captured = false;
        console_state.insert("captured".into(), Value::Bool(captured));
        env.details
            .insert("console".into(), Value::Object(console_state));

        let background = Background {
            shared: shared.clone(),
            dir: dir.clone(),
            lock,
            writer,
            init,
            client,
            interval: opts.batch_interval,
            python_version: opts.python_version,
            sdk_version: opts.sdk_version,
            env,
        };
        let thread = match std::thread::Builder::new()
            .name("losscape-run".into())
            .spawn(move || background.run())
        {
            Ok(thread) => thread,
            Err(err) => {
                #[cfg(unix)]
                if let Some(capture) = capture {
                    capture.stop(Duration::ZERO);
                }
                return Err(local(err));
            }
        };

        let sampler = opts.system_interval.and_then(|interval| {
            let shared = shared.clone();
            let push = move |points: &[(String, f64)], time_ms: i64, step: u64| {
                let mut buffer = shared.buffer.lock().expect("not poisoned");
                for (key, value) in points {
                    buffer.push_untracked(key, step, time_ms, *value);
                }
            };
            Sampler::start(interval, push)
                .inspect_err(|err| eprintln!("losscape: system metrics are off: {err}"))
                .ok()
        });
        Ok(Self {
            shared,
            thread: Mutex::new(Some(thread)),
            #[cfg(unix)]
            capture: Mutex::new(capture),
            sampler: Mutex::new(sampler),
            owner_pid: std::process::id(),
            run_id,
            dir,
            next_step,
        })
    }

    pub fn id(&self) -> RunId {
        self.run_id
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Someone asked for this run to stop (known from its heartbeats).
    pub fn abort_requested(&self) -> bool {
        self.shared.abort.load(Ordering::Relaxed)
    }

    /// For a resumed run: the first step after what was already logged.
    pub fn next_step(&self) -> Option<u64> {
        self.next_step
    }

    /// Direct access for bindings that push many points under one lock.
    pub fn buffer(&self) -> MutexGuard<'_, Buffer> {
        self.shared.buffer.lock().expect("not poisoned")
    }

    /// Call after pushing; starts a batch early when a lot is buffered.
    pub fn after_push(&self, rows: usize) {
        if rows >= WAKE_ROWS {
            self.shared.wake.notify_one();
        }
    }

    pub fn log(&self, step: u64, points: &[(&str, f64)]) {
        let now = now_ms();
        let rows = {
            let mut buffer = self.buffer();
            for &(key, value) in points {
                buffer.push(key, step, now, value);
            }
            buffer.rows()
        };
        self.after_push(rows);
    }

    pub fn update_summary(&self, mut update: Map<String, Value>) {
        crate::redact::redact_json(&mut update);
        self.buffer().update_summary(update);
    }

    pub fn update_config(&self, mut update: Map<String, Value>) {
        crate::redact::redact_json(&mut update);
        self.buffer().update_config(update);
    }

    /// Records what a model is made of, replacing earlier descriptions of the same name.
    pub fn update_models(&self, mut update: Map<String, Value>) {
        crate::redact::redact_json(&mut update);
        self.buffer().update_models(update);
    }

    /// Flushes everything to the local log, then waits up to `timeout` for the upload.
    /// Later calls return the first outcome.
    pub fn finish(&self, state: RunExitState, exit_code: i32, timeout: Duration) -> FinishOutcome {
        if std::process::id() != self.owner_pid {
            return FinishOutcome::Failed("this run belongs to the parent process".into());
        }
        let Some(thread) = self.thread.lock().expect("not poisoned").take() else {
            return self
                .wait_outcome(None)
                .unwrap_or_else(|| FinishOutcome::Failed("run finished".into()));
        };
        if let Some(sampler) = self.sampler.lock().expect("not poisoned").take() {
            sampler.stop();
        }
        // Output written before finish is in the last batch.
        #[cfg(unix)]
        if let Some(capture) = self.capture.lock().expect("not poisoned").take() {
            capture.stop(CONSOLE_DRAIN);
        }
        *self.shared.finish.lock().expect("not poisoned") = Some(RunFinish {
            run_id: self.run_id.as_bytes().to_vec(),
            state: state as i32,
            exit_code,
            client_time_ms: now_ms(),
            last_seq: 0, // set by the batcher
        });
        self.shared.wake.notify_one();

        if self.wait_outcome(Some(timeout)).is_none() {
            self.shared.cancel.notify_one();
        }
        thread.join().ok();
        self.wait_outcome(None)
            .expect("set before the thread exits")
    }

    fn wait_outcome(&self, timeout: Option<Duration>) -> Option<FinishOutcome> {
        let outcome = self.shared.outcome.lock().expect("not poisoned");
        let outcome = match timeout {
            Some(timeout) => {
                self.shared
                    .done
                    .wait_timeout_while(outcome, timeout, |o| o.is_none())
                    .expect("not poisoned")
                    .0
            }
            None => outcome,
        };
        outcome.clone()
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if std::process::id() != self.owner_pid {
            // A forked child: the background thread only exists in the parent.
            if let Some(thread) = self.thread.get_mut().ok().and_then(Option::take) {
                std::mem::forget(thread);
            }
            return;
        }
        // Not finished explicitly: keep everything locally; `sync` uploads it later.
        self.finish(RunExitState::Killed, -1, Duration::ZERO);
    }
}

struct Background {
    shared: Arc<Shared>,
    dir: PathBuf,
    lock: RunLock,
    writer: WalWriter,
    init: RunInit,
    client: Option<Client>,
    interval: Duration,
    python_version: String,
    sdk_version: String,
    env: Extra,
}

impl Background {
    fn run(mut self) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let outcome = runtime.block_on(self.work());
        drop(runtime);

        let Self {
            shared, dir, lock, ..
        } = self;
        drop(lock);
        if outcome == FinishOutcome::Uploaded {
            fs::remove_dir_all(&dir).ok();
        }
        *shared.outcome.lock().expect("not poisoned") = Some(outcome);
        shared.done.notify_all();
    }

    async fn work(&mut self) -> FinishOutcome {
        self.init.environment = Some(crate::env::capture(
            &self.python_version,
            &self.sdk_version,
            &self.env,
        ));
        if let Err(err) = wal::write_atomic(&self.dir.join(INIT_FILE), &self.init.encode_to_vec()) {
            return FinishOutcome::Failed(format!("writing local log: {err}"));
        }
        let shared = &self.shared;
        let batcher = batch_loop(
            shared,
            &mut self.writer,
            &self.dir,
            &self.init.run_id,
            self.interval,
        );

        let Some(client) = &self.client else {
            return match batcher.await {
                Ok(()) => FinishOutcome::SavedLocally {
                    pending_batches: pending(shared),
                    reason: "offline mode".into(),
                },
                Err(err) => FinishOutcome::Failed(format!("writing local log: {err}")),
            };
        };
        let uploader = async {
            let result = tokio::select! {
                result = upload_run(client, &self.dir, Some(&shared.live)) => result.map(|()| true),
                _ = heartbeat(client, &self.init.run_id, &self.shared.abort) => unreachable!("heartbeat never ends"),
                _ = shared.cancel.notified() => Ok(false),
            };
            if let Err(err) = &result {
                eprintln!(
                    "losscape: upload stopped: {err}. Logging continues to {}; upload it later with `losscape sync`.",
                    self.dir.display()
                );
            }
            result
        };
        match tokio::join!(batcher, uploader) {
            (Err(err), _) => FinishOutcome::Failed(format!("writing local log: {err}")),
            (Ok(()), Ok(true)) => FinishOutcome::Uploaded,
            (Ok(()), Ok(false)) => FinishOutcome::SavedLocally {
                pending_batches: pending(shared),
                reason: client
                    .last_error
                    .lock()
                    .expect("not poisoned")
                    .clone()
                    .unwrap_or_else(|| "upload timed out".into()),
            },
            (Ok(()), Err(err)) => FinishOutcome::Failed(err.to_string()),
        }
    }
}

fn pending(shared: &Shared) -> u64 {
    shared
        .written
        .load(Ordering::SeqCst)
        .saturating_sub(shared.live.acked.load(Ordering::SeqCst))
}

/// Every `interval` (or when woken), turns the buffer into one batch in the local log.
/// Returns after the final batch once finish is requested.
async fn batch_loop(
    shared: &Shared,
    writer: &mut WalWriter,
    dir: &Path,
    run_id: &[u8],
    interval: Duration,
) -> io::Result<()> {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = shared.wake.notified() => {}
        }
        // Taken before draining, so every point logged before finish() is included.
        let finish = shared.finish.lock().expect("not poisoned").take();
        // Several batches when the console produced a lot since the last one.
        loop {
            let console = shared.console.lock().expect("not poisoned").take(
                Instant::now(),
                finish.is_some(),
                MAX_LINES_PER_BATCH,
            );
            let more = console.len() == MAX_LINES_PER_BATCH;
            let drained = {
                let mut buffer = shared.buffer.lock().expect("not poisoned");
                (!buffer.is_empty() || !console.is_empty()).then(|| buffer.drain())
            };
            let Some(drained) = drained else { break };
            let batch = LogBatch {
                run_id: run_id.to_vec(),
                seq: writer.next_seq(),
                confirmed_below: shared.live.acked.load(Ordering::SeqCst),
                scalars: Some(drained.scalars),
                summary_json: json_object(drained.summary),
                config_json: json_object(drained.config),
                models_json: json_object(drained.models),
                console,
            };
            let payload = zstd::bulk::compress(&batch.encode_to_vec(), ZSTD_LEVEL)?;
            writer.append(&payload)?;
            shared.written.store(writer.next_seq(), Ordering::SeqCst);
            shared.live.notify.notify_one();
            if !more {
                break;
            }
        }
        if let Some(mut finish) = finish {
            finish.last_seq = writer.next_seq().saturating_sub(1);
            wal::write_atomic(&dir.join(FINISH_FILE), &finish.encode_to_vec())?;
            shared.live.finished.store(true, Ordering::SeqCst);
            shared.live.notify.notify_one();
            return Ok(());
        }
    }
}

fn json_object(map: Map<String, Value>) -> String {
    if map.is_empty() {
        String::new()
    } else {
        Value::Object(map).to_string()
    }
}

/// Best effort, so a run that logs rarely isn't marked as crashed. The answer also says
/// whether someone asked this run to stop.
async fn heartbeat(client: &Client, run_id: &[u8], abort: &AtomicBool) {
    let body = losscape_core::proto::v0::Heartbeat {
        run_id: run_id.to_vec(),
        client_time_ms: 0,
    };
    loop {
        tokio::time::sleep(HEARTBEAT_EVERY).await;
        let body = losscape_core::proto::v0::Heartbeat {
            client_time_ms: now_ms(),
            ..body.clone()
        };
        let answer = tokio::time::timeout(
            Duration::from_secs(10),
            client.post("runs/heartbeat", body.encode_to_vec().into(), false),
        )
        .await;
        if let Ok(Ok(bytes)) = answer
            && Ack::decode(bytes).is_ok_and(|ack| ack.abort)
        {
            abort.store(true, Ordering::Relaxed);
        }
    }
}

/// Asks the server where a resumed run stopped. Runs on its own thread, so it works
/// whether or not the caller is inside an async runtime.
fn fetch_resume_point(
    server: &str,
    api_key: &str,
    init: &RunInit,
) -> Result<RunInitResponse, UploadError> {
    let client = Client::new(server, api_key, Some(RESUME_TIMEOUT))?;
    let body = RunInit {
        resume: true,
        ..init.clone()
    }
    .encode_to_vec();
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                let response = runtime.block_on(client.post("runs/init", body.into(), false))?;
                Ok(RunInitResponse::decode(response)?)
            })
            .join()
            .expect("resume thread panicked")
    })
}

#[derive(Default)]
struct Logged {
    /// Highest step of a metric (system metrics aside: their step is the time).
    max_step: Option<u64>,
    next_console_line: u64,
}

/// What a run's local log holds (for resuming after a crash).
fn logged_so_far(dir: &Path) -> Logged {
    let mut reader = WalReader::new(dir, 0);
    let mut logged = Logged::default();
    while let Ok(Some((_, payload))) = reader.next_batch() {
        let Ok(raw) = zstd::decode_all(payload.as_slice()) else {
            continue;
        };
        let Ok(batch) = LogBatch::decode(raw.as_slice()) else {
            continue;
        };
        if let Some(s) = batch.scalars {
            for (i, &step) in s.step.iter().enumerate() {
                if !s.keys[s.key_idx[i] as usize].starts_with(SYSTEM_PREFIX) {
                    logged.max_step = logged.max_step.max(Some(step));
                }
            }
        }
        if let Some(last) = batch.console.iter().map(|l| l.line).max() {
            logged.next_console_line = logged.next_console_line.max(last + 1);
        }
    }
    logged
}
