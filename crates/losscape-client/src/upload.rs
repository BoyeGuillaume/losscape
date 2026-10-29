//! Sends a run's local log to the server: init, batches (several in flight), finish.
//! Used both while a run is live (following the log as it grows) and by `sync`.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use losscape_core::proto::v0::{Error as ProtoError, RunExitState, RunFinish, RunInit};
use losscape_core::{PROTOCOL_HEADER, PROTOCOL_VERSION};
use prost::Message;
use reqwest::{StatusCode, header};
use tokio::sync::Notify;
use tokio::task::JoinSet;

use crate::wal::{self, FINISH_FILE, INIT_FILE, WalReader};

/// Unconfirmed batches per run. The server's dedup window allows far more (4096).
const MAX_IN_FLIGHT: usize = 8;
const MAX_BACKOFF: Duration = Duration::from_secs(10);
/// How often the confirmed position is saved to disk (it only bounds re-sends).
const SAVE_ACKED_EVERY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, thiserror::Error)]
pub enum UploadError {
    #[error("server rejected {path}: {code}: {message}")]
    Rejected {
        path: String,
        code: String,
        message: String,
    },
    #[error("server unreachable: {0}")]
    Unreachable(String),
    #[error("local log: {0}")]
    Local(String),
}

impl From<io::Error> for UploadError {
    fn from(err: io::Error) -> Self {
        Self::Local(err.to_string())
    }
}

impl From<prost::DecodeError> for UploadError {
    fn from(err: prost::DecodeError) -> Self {
        Self::Local(format!("corrupt file: {err}"))
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    api_key: String,
    /// Give up retrying after this long (`None`: retry until cancelled).
    retry_for: Option<Duration>,
    /// Why the last attempt failed, for user-facing messages.
    pub last_error: Arc<Mutex<Option<String>>>,
}

enum Attempt {
    Retry(String),
    Fatal(UploadError),
}

impl Client {
    pub fn new(
        server: &str,
        api_key: &str,
        retry_for: Option<Duration>,
    ) -> Result<Self, UploadError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| UploadError::Local(format!("HTTP client: {e}")))?;
        Ok(Self {
            http,
            base: format!("{}/api/v0", server.trim_end_matches('/')),
            api_key: api_key.to_owned(),
            retry_for,
            last_error: Arc::default(),
        })
    }

    /// POSTs a protobuf body, retrying network errors and 5xx/429 with backoff.
    pub async fn post(&self, path: &str, body: Bytes, zstd: bool) -> Result<Bytes, UploadError> {
        let started = Instant::now();
        let mut delay = Duration::from_millis(100);
        loop {
            match self.attempt(path, body.clone(), zstd).await {
                Ok(response) => {
                    *self.last_error.lock().expect("not poisoned") = None;
                    return Ok(response);
                }
                Err(Attempt::Fatal(err)) => return Err(err),
                Err(Attempt::Retry(reason)) => {
                    *self.last_error.lock().expect("not poisoned") = Some(reason.clone());
                    if self
                        .retry_for
                        .is_some_and(|limit| started.elapsed() >= limit)
                    {
                        return Err(UploadError::Unreachable(reason));
                    }
                }
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(MAX_BACKOFF);
        }
    }

    async fn attempt(&self, path: &str, body: Bytes, zstd: bool) -> Result<Bytes, Attempt> {
        let mut req = self
            .http
            .post(format!("{}/{path}", self.base))
            .bearer_auth(&self.api_key)
            .header(header::CONTENT_TYPE, "application/x-protobuf")
            .header(PROTOCOL_HEADER, PROTOCOL_VERSION.to_string())
            .body(body);
        if zstd {
            req = req.header(header::CONTENT_ENCODING, "zstd");
        }
        let res = req.send().await.map_err(|e| Attempt::Retry(describe(&e)))?;
        let status = res.status();
        let bytes = res
            .bytes()
            .await
            .map_err(|e| Attempt::Retry(describe(&e)))?;
        if status.is_success() {
            return Ok(bytes);
        }
        let error = ProtoError::decode(bytes).unwrap_or_default();
        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(Attempt::Retry(format!("HTTP {status} {}", error.code)));
        }
        Err(Attempt::Fatal(UploadError::Rejected {
            path: path.to_owned(),
            code: if error.code.is_empty() {
                status.to_string()
            } else {
                error.code
            },
            message: error.message,
        }))
    }
}

fn describe(err: &reqwest::Error) -> String {
    let mut msg = err.to_string();
    let mut source = std::error::Error::source(err);
    while let Some(inner) = source {
        msg = format!("{msg}: {inner}");
        source = inner.source();
    }
    msg
}

/// Signals between a live run's batcher and its uploader.
#[derive(Clone, Default)]
pub struct Live {
    /// New batch appended, or the run finished.
    pub notify: Arc<Notify>,
    /// The batcher wrote its last batch and `finish.pb`.
    pub finished: Arc<AtomicBool>,
    /// Every seq below this is confirmed (sent to the server as `confirmed_below`).
    pub acked: Arc<AtomicU64>,
}

/// Uploads everything in `dir` and closes the run on the server. Live: follows the log
/// until the batcher finishes. Without `live` (sync): uploads what is there; a run with
/// no `finish.pb` belonged to a process that died, so it is closed as killed.
/// The caller deletes `dir` afterwards.
pub async fn upload_run(
    client: &Client,
    dir: &Path,
    live: Option<&Live>,
) -> Result<(), UploadError> {
    let mut init = RunInit::decode(std::fs::read(dir.join(INIT_FILE))?.as_slice())?;
    // Creates the run, or attaches to it if an earlier attempt already did.
    init.resume = true;
    let run_id = init.run_id.clone();
    client
        .post("runs/init", init.encode_to_vec().into(), false)
        .await?;

    let mut acked = wal::read_acked(dir)?;
    let mut reader = WalReader::new(dir, acked);
    let mut unconfirmed = BTreeSet::new();
    let mut next_unsent = acked;
    let mut in_flight = JoinSet::new();
    let mut saved = Instant::now();
    let notified = || async {
        match live {
            Some(live) => live.notify.notified().await,
            None => std::future::pending().await,
        }
    };

    loop {
        // Read before checking for new data, so a batch written just before `finished`
        // was set is never missed.
        let finished = live.is_none_or(|l| l.finished.load(Ordering::SeqCst));
        while in_flight.len() < MAX_IN_FLIGHT {
            let Some((seq, payload)) = reader.next_batch()? else {
                break;
            };
            unconfirmed.insert(seq);
            next_unsent = seq + 1;
            let client = client.clone();
            in_flight
                .spawn(async move { (seq, client.post("ingest", payload.into(), true).await) });
        }
        if in_flight.is_empty() {
            if finished {
                break;
            }
            tokio::time::timeout(Duration::from_secs(1), notified())
                .await
                .ok();
            continue;
        }
        tokio::select! {
            done = in_flight.join_next() => {
                let (seq, result) = done.expect("not empty").expect("upload task panicked");
                result?;
                unconfirmed.remove(&seq);
                let floor = unconfirmed.first().copied().unwrap_or(next_unsent);
                if floor > acked {
                    acked = floor;
                    if let Some(live) = live {
                        live.acked.store(acked, Ordering::SeqCst);
                    }
                    if saved.elapsed() >= SAVE_ACKED_EVERY {
                        wal::write_acked(dir, acked)?;
                        saved = Instant::now();
                    }
                }
            }
            _ = notified() => {}
        }
    }
    wal::write_acked(dir, acked)?;

    let finish = match std::fs::read(dir.join(FINISH_FILE)) {
        Ok(bytes) => RunFinish::decode(bytes.as_slice())?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => RunFinish {
            run_id,
            state: RunExitState::Killed as i32,
            exit_code: -1,
            last_seq: next_unsent.saturating_sub(1),
            ..Default::default()
        },
        Err(e) => return Err(e.into()),
    };
    client
        .post("runs/finish", finish.encode_to_vec().into(), false)
        .await?;
    Ok(())
}
