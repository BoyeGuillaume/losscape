//! Simulates many training runs logging scalars at once, the way the SDK will:
//! a few batches in flight per run, retries with backoff until each batch is confirmed.
//!
//!     cargo run --release -p losscape-loadgen -- --runs 32 --metrics 32 --steps 20000

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use clap::Parser;
use losscape_core::ids::RunId;
use losscape_core::proto::v0::{Ack, Error, LogBatch, RunInit, RunInitResponse, ScalarColumns};
use losscape_core::{PROTOCOL_HEADER, PROTOCOL_VERSION};
use prost::Message;
use reqwest::{StatusCode, header};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// Give up on a request after retrying this long (covers a server restart).
const RETRY_FOR: Duration = Duration::from_secs(120);

#[derive(Parser, Clone, Debug)]
struct Args {
    #[arg(long, default_value = "http://localhost:8080")]
    server: String,
    #[arg(long, env = "LOSSCAPE_API_KEY")]
    api_key: String,
    #[arg(long, default_value = "loadgen")]
    project: String,
    /// Concurrent runs.
    #[arg(long, default_value_t = 16)]
    runs: u64,
    /// Scalar metrics logged at every step.
    #[arg(long, default_value_t = 32)]
    metrics: u64,
    /// Steps per run.
    #[arg(long, default_value_t = 20_000)]
    steps: u64,
    /// Steps per batch (the SDK batches ~100 ms of logging).
    #[arg(long, default_value_t = 100)]
    steps_per_batch: u64,
    /// Unconfirmed batches allowed per run.
    #[arg(long, default_value_t = 4)]
    in_flight: usize,
    /// zstd level; 0 sends uncompressed bodies.
    #[arg(long, default_value_t = 1)]
    zstd_level: i32,
    /// Write a JSON report (run ids, totals) to this path.
    #[arg(long)]
    report: Option<std::path::PathBuf>,
}

#[derive(Default)]
struct RunStats {
    latencies: Vec<Duration>,
    retries: u64,
    duplicates: u64,
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    dotenvy::dotenv().ok();
    let args = Args::parse();
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(1024)
        .build()?;
    let points_per_run = args.steps * args.metrics;
    println!(
        "{} runs × {} metrics × {} steps = {} points, {} steps/batch, {} in flight/run, zstd {}",
        args.runs,
        args.metrics,
        args.steps,
        args.runs * points_per_run,
        args.steps_per_batch,
        args.in_flight,
        args.zstd_level
    );

    let run_ids: Vec<RunId> = (0..args.runs).map(|_| RunId::new()).collect();
    let started = Instant::now();
    let mut runs = JoinSet::new();
    for &run_id in &run_ids {
        runs.spawn(simulate_run(client.clone(), args.clone(), run_id));
    }
    let mut total = RunStats::default();
    while let Some(stats) = runs.join_next().await {
        let stats = stats??;
        total.latencies.extend(stats.latencies);
        total.retries += stats.retries;
        total.duplicates += stats.duplicates;
    }
    let elapsed = started.elapsed();

    let points = args.runs * points_per_run;
    let rate = points as f64 / elapsed.as_secs_f64();
    total.latencies.sort();
    let pct = |p: f64| {
        let i = ((total.latencies.len() as f64 * p) as usize).min(total.latencies.len() - 1);
        total.latencies[i].as_secs_f64() * 1e3
    };
    let (p50, p99, max) = (pct(0.50), pct(0.99), pct(1.0));
    println!(
        "{points} points in {:.2}s = {:.0} points/s | confirm latency p50 {p50:.1} ms, p99 {p99:.1} ms, max {max:.1} ms | {} batches, {} retries, {} duplicates",
        elapsed.as_secs_f64(),
        rate,
        total.latencies.len(),
        total.retries,
        total.duplicates,
    );

    if let Some(path) = &args.report {
        let report = serde_json::json!({
            "project": args.project,
            "run_ids": run_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
            "points_per_run": points_per_run,
            "points": points,
            "elapsed_s": elapsed.as_secs_f64(),
            "points_per_s": rate,
            "latency_ms": {"p50": p50, "p99": p99, "max": max},
            "retries": total.retries,
            "duplicates": total.duplicates,
        });
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

async fn simulate_run(
    client: reqwest::Client,
    args: Args,
    run_id: RunId,
) -> Result<RunStats, BoxError> {
    let init = RunInit {
        run_id: run_id.as_bytes().to_vec(),
        project: args.project.clone(),
        config_json: format!(
            r#"{{"metrics": {}, "steps": {}}}"#,
            args.metrics, args.steps
        ),
        ..Default::default()
    };
    let (body, _, _) = send(
        &client,
        &args,
        "runs/init",
        init.encode_to_vec().into(),
        false,
    )
    .await?;
    RunInitResponse::decode(body)?;

    let keys: Vec<String> = (0..args.metrics)
        .map(|m| format!("train/metric_{m:03}"))
        .collect();
    let window = Arc::new(Semaphore::new(args.in_flight));
    // Seqs sent but not yet confirmed; the smallest is sent as `confirmed_below`.
    let unconfirmed = Arc::new(Mutex::new(BTreeSet::new()));
    let mut batches = JoinSet::new();
    let mut stats = RunStats::default();

    for (seq, first_step) in (0..args.steps)
        .step_by(args.steps_per_batch as usize)
        .enumerate()
    {
        let seq = seq as u64;
        let permit = window.clone().acquire_owned().await?;
        let confirmed_below = {
            let mut unconfirmed = unconfirmed.lock().expect("not poisoned");
            unconfirmed.insert(seq);
            *unconfirmed.first().expect("just inserted")
        };
        let last_step = (first_step + args.steps_per_batch).min(args.steps);
        let steps = first_step..last_step;
        let body = encode_batch(run_id, seq, confirmed_below, &keys, steps, args.zstd_level)?;
        let (client, args, unconfirmed) = (client.clone(), args.clone(), unconfirmed.clone());
        batches.spawn(async move {
            let result = send(&client, &args, "ingest", body, args.zstd_level > 0).await;
            if result.is_ok() {
                unconfirmed.lock().expect("not poisoned").remove(&seq);
            }
            drop(permit);
            result
        });
        while let Some(done) = batches.try_join_next() {
            record(&mut stats, done??)?;
        }
    }
    while let Some(done) = batches.join_next().await {
        record(&mut stats, done??)?;
    }
    Ok(stats)
}

fn record(
    stats: &mut RunStats,
    (body, latency, retries): (Bytes, Duration, u64),
) -> Result<(), BoxError> {
    let ack = Ack::decode(body)?;
    stats.latencies.push(latency);
    stats.retries += retries;
    stats.duplicates += ack.duplicate as u64;
    Ok(())
}

fn encode_batch(
    run_id: RunId,
    seq: u64,
    confirmed_below: u64,
    keys: &[String],
    steps: std::ops::Range<u64>,
    zstd_level: i32,
) -> Result<Bytes, BoxError> {
    let rows = (steps.end - steps.start) as usize * keys.len();
    let mut cols = ScalarColumns {
        keys: keys.to_vec(),
        key_idx: Vec::with_capacity(rows),
        step: Vec::with_capacity(rows),
        time_ms: Vec::with_capacity(rows),
        value: Vec::with_capacity(rows),
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    for step in steps {
        for k in 0..keys.len() {
            cols.key_idx.push(k as u32);
            cols.step.push(step);
            cols.time_ms.push(now_ms);
            // A noisy decaying curve, like a loss.
            let s = step as f64;
            cols.value
                .push((1.0 + k as f64) / (1.0 + s / 500.0) + 0.05 * (s * 0.37 + k as f64).sin());
        }
    }
    let batch = LogBatch {
        run_id: run_id.as_bytes().to_vec(),
        seq,
        confirmed_below,
        scalars: Some(cols),
        ..Default::default()
    };
    let bytes = batch.encode_to_vec();
    Ok(if zstd_level > 0 {
        zstd::encode_all(bytes.as_slice(), zstd_level)?.into()
    } else {
        bytes.into()
    })
}

/// POST with retries on connection errors and 5xx. Returns (body, latency of the
/// successful attempt, retries).
async fn send(
    client: &reqwest::Client,
    args: &Args,
    path: &str,
    body: Bytes,
    zstd: bool,
) -> Result<(Bytes, Duration, u64), BoxError> {
    let url = format!("{}/api/v0/{path}", args.server.trim_end_matches('/'));
    let first_attempt = Instant::now();
    let mut retries = 0;
    loop {
        let mut req = client
            .post(&url)
            .bearer_auth(&args.api_key)
            .header(header::CONTENT_TYPE, "application/x-protobuf")
            .header(PROTOCOL_HEADER, PROTOCOL_VERSION.to_string())
            .body(body.clone());
        if zstd {
            req = req.header(header::CONTENT_ENCODING, "zstd");
        }
        let attempt = Instant::now();
        let retryable = match req.send().await {
            Ok(res) if res.status().is_success() => {
                return Ok((res.bytes().await?, attempt.elapsed(), retries));
            }
            Ok(res)
                if res.status().is_server_error()
                    || res.status() == StatusCode::TOO_MANY_REQUESTS =>
            {
                format!("HTTP {}", res.status())
            }
            Ok(res) => {
                let status = res.status();
                let error = Error::decode(res.bytes().await?).unwrap_or_default();
                return Err(
                    format!("{path}: HTTP {status} {}: {}", error.code, error.message).into(),
                );
            }
            Err(err) => err.to_string(),
        };
        if first_attempt.elapsed() > RETRY_FOR {
            return Err(format!("{path}: giving up after {retries} retries: {retryable}").into());
        }
        retries += 1;
        tokio::time::sleep(Duration::from_millis(50 * (1 << retries.min(5)))).await;
    }
}
