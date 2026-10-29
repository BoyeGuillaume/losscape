//! Single background task that writes queued batches in large chunks.
//!
//! Per flush: drop batches already stored (seq window from Postgres), write the rest to
//! ClickHouse in one insert, then advance the seq windows and merge summary/config in
//! Postgres, and only then answer the waiting requests. If the server dies between the
//! two writes, the client re-sends and ReplacingMergeTree collapses the duplicate rows.
//!
//! One flusher per server keeps the seq windows consistent without locks; running several
//! replicas will need runs partitioned across them.

use std::collections::{HashMap, HashSet};

use axum::http::StatusCode;
use serde_json::{Map, Value};
use sqlx::PgPool;
use sqlx::types::Json;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep_until};
use uuid::Uuid;

use super::Batch;
use super::window::{Insert, MAX_AHEAD, SeqWindow};
use crate::config::IngestConfig;
use crate::error::ApiError;

/// `Ok(true)` when the batch had already been stored.
type Reply = oneshot::Sender<Result<bool, ApiError>>;

struct Pending {
    batch: Batch,
    reply: Reply,
}

#[derive(Clone)]
pub struct IngestQueue {
    tx: mpsc::Sender<Pending>,
}

impl IngestQueue {
    /// Waits until the batch is stored. Waits for queue space when the server is
    /// saturated, which slows clients down instead of dropping data.
    pub async fn submit(&self, batch: Batch) -> Result<bool, ApiError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Pending { batch, reply })
            .await
            .map_err(|_| ApiError::unavailable("server is shutting down"))?;
        rx.await.map_err(|_| ApiError::internal())?
    }
}

/// The task ends once every [`IngestQueue`] handle is dropped and the queue is drained.
pub fn spawn_flusher(
    pg: PgPool,
    ch: clickhouse::Client,
    config: IngestConfig,
) -> (IngestQueue, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(config.queue_capacity);
    let handle = tokio::spawn(run(pg, ch, config, rx));
    (IngestQueue { tx }, handle)
}

async fn run(
    pg: PgPool,
    ch: clickhouse::Client,
    config: IngestConfig,
    mut rx: mpsc::Receiver<Pending>,
) {
    while let Some(first) = rx.recv().await {
        let deadline = Instant::now() + config.flush_interval;
        let mut rows = first.batch.rows();
        let mut chunk = vec![first];
        while rows < config.flush_max_rows {
            tokio::select! {
                biased;
                next = rx.recv() => match next {
                    Some(p) => {
                        rows += p.batch.rows();
                        chunk.push(p);
                    }
                    None => break,
                },
                _ = sleep_until(deadline) => break,
            }
        }
        flush(&pg, &ch, chunk).await;
    }
    tracing::info!("ingest queue drained");
}

/// An accepted batch plus every request waiting on it (retries that arrived in the same
/// chunk share the original's outcome).
struct Accepted {
    batch: Batch,
    replies: Vec<(Reply, bool)>,
}

async fn flush(pg: &PgPool, ch: &clickhouse::Client, chunk: Vec<Pending>) {
    let started = std::time::Instant::now();
    let run_ids: Vec<Uuid> = chunk
        .iter()
        .map(|p| p.batch.run_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let mut windows = match load_windows(pg, &run_ids).await {
        Ok(w) => w,
        Err(err) => {
            let err = ApiError::from(err);
            for p in chunk {
                let _ = p.reply.send(Err(err.clone()));
            }
            return;
        }
    };

    let mut accepted: Vec<Accepted> = Vec::new();
    let mut index: HashMap<(Uuid, u64), usize> = HashMap::new();
    let mut duplicates = 0usize;
    for Pending { batch, reply } in chunk {
        let key = (batch.run_id, batch.seq);
        if let Some(&i) = index.get(&key) {
            accepted[i].replies.push((reply, true));
            continue;
        }
        let Some(window) = windows.get_mut(&batch.run_id) else {
            let _ = reply.send(Err(ApiError::unknown_run()));
            continue;
        };
        window.advance_to(batch.confirmed_below);
        match window.insert(batch.seq) {
            Insert::New => {
                index.insert(key, accepted.len());
                accepted.push(Accepted {
                    batch,
                    replies: vec![(reply, false)],
                });
            }
            Insert::Duplicate => {
                duplicates += 1;
                let _ = reply.send(Ok(true));
            }
            Insert::TooFarAhead => {
                let _ = reply.send(Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "seq_too_far_ahead",
                    format!("seq is more than {MAX_AHEAD} ahead of the oldest unstored batch"),
                )));
            }
        }
    }
    if accepted.is_empty() {
        return;
    }

    let rows: usize = accepted.iter().map(|a| a.batch.rows()).sum();
    let mut clickhouse_ms = 0;
    let result = async {
        let ch_started = std::time::Instant::now();
        write_metrics(ch, &accepted).await?;
        clickhouse_ms = ch_started.elapsed().as_millis() as u64;
        update_runs(pg, &accepted, &windows).await?;
        Ok::<_, ApiError>(())
    }
    .await;

    tracing::debug!(
        batches = accepted.len(),
        duplicates,
        rows,
        elapsed_ms = started.elapsed().as_millis() as u64,
        clickhouse_ms,
        ok = result.is_ok(),
        "flushed"
    );
    for a in accepted {
        for (reply, duplicate) in a.replies {
            let _ = reply.send(result.clone().map(|()| duplicate));
        }
    }
}

async fn load_windows(pg: &PgPool, run_ids: &[Uuid]) -> sqlx::Result<HashMap<Uuid, SeqWindow>> {
    let rows: Vec<(Uuid, i64, Vec<i64>)> =
        sqlx::query_as("SELECT id, next_seq, pending_seqs FROM runs WHERE id = ANY($1)")
            .bind(run_ids)
            .fetch_all(pg)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, next, pending)| (id, SeqWindow::from_db(next, &pending)))
        .collect())
}

#[derive(clickhouse::Row, serde::Serialize)]
struct MetricRow<'a> {
    project_id: u64,
    #[serde(with = "clickhouse::serde::uuid")]
    run_id: Uuid,
    key: &'a str,
    step: u64,
    ts: i64, // DateTime64(3): milliseconds since epoch
    value: f64,
    seq: u64,
}

async fn write_metrics(
    ch: &clickhouse::Client,
    accepted: &[Accepted],
) -> clickhouse::error::Result<()> {
    if accepted.iter().all(|a| a.batch.rows() == 0) {
        return Ok(());
    }
    let mut insert = ch.insert::<MetricRow<'_>>("metrics")?;
    for Accepted { batch, .. } in accepted {
        let s = &batch.scalars;
        for i in 0..s.value.len() {
            insert
                .write(&MetricRow {
                    project_id: batch.project_id as u64,
                    run_id: batch.run_id,
                    key: &s.keys[s.key_idx[i] as usize],
                    step: s.step[i],
                    ts: s.time_ms[i],
                    value: s.value[i],
                    seq: batch.seq,
                })
                .await?;
        }
    }
    insert.end().await
}

/// Summary/config changes for one run, merged across the batches of a flush.
struct RunUpdate {
    run_id: Uuid,
    summary: Option<Map<String, Value>>,
    config: Option<Map<String, Value>>,
}

async fn update_runs(
    pg: &PgPool,
    accepted: &[Accepted],
    windows: &HashMap<Uuid, SeqWindow>,
) -> sqlx::Result<()> {
    // Merge summary/config updates per run in seq order, so the latest batch wins.
    let mut ordered: Vec<&Batch> = accepted.iter().map(|a| &a.batch).collect();
    ordered.sort_by_key(|b| (b.run_id, b.seq));
    let mut merged: Vec<RunUpdate> = Vec::new();
    for batch in ordered {
        if merged.last().is_none_or(|u| u.run_id != batch.run_id) {
            merged.push(RunUpdate {
                run_id: batch.run_id,
                summary: None,
                config: None,
            });
        }
        let update = merged.last_mut().expect("pushed above");
        merge_into(&mut update.summary, batch.summary_json.as_deref());
        merge_into(&mut update.config, batch.config_json.as_deref());
    }

    // Runs are updated in id order so concurrent transactions can't deadlock.
    let mut tx = pg.begin().await?;
    // Don't wait for the WAL fsync: under heavy disk writes (ClickHouse merges on the same
    // disk) it took up to 18 s per commit. A server crash loses nothing; a Postgres or OS
    // crash can lose ~0.6 s of window/summary updates. Lost window updates are repaired by
    // the client's `confirmed_below`, and the points themselves are already in ClickHouse.
    sqlx::query("SET LOCAL synchronous_commit = off")
        .execute(&mut *tx)
        .await?;
    for RunUpdate {
        run_id,
        summary,
        config,
    } in merged
    {
        let (next_seq, pending) = windows[&run_id].to_db();
        sqlx::query(
            "UPDATE runs SET next_seq = $2, pending_seqs = $3, heartbeat_at = now(),
                 summary = summary || COALESCE($4, '{}'::jsonb),
                 config = config || COALESCE($5, '{}'::jsonb)
             WHERE id = $1",
        )
        .bind(run_id)
        .bind(next_seq)
        .bind(pending)
        .bind(summary.map(Json))
        .bind(config.map(Json))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

fn merge_into(target: &mut Option<Map<String, Value>>, raw: Option<&str>) {
    // Validated as a JSON object by the request handler.
    if let Some(Ok(Value::Object(update))) = raw.map(serde_json::from_str::<Value>) {
        target.get_or_insert_with(Map::new).extend(update);
    }
}

#[cfg(test)]
mod tests {
    use super::merge_into;

    #[test]
    fn later_updates_win() {
        let mut target = None;
        merge_into(&mut target, Some(r#"{"loss": 1, "acc": 0.5}"#));
        merge_into(&mut target, None);
        merge_into(&mut target, Some(r#"{"loss": 2}"#));
        let target = serde_json::Value::Object(target.unwrap());
        assert_eq!(target, serde_json::json!({"loss": 2, "acc": 0.5}));
    }
}
