# losscape — Requirements

A free, self-hosted, high-performance ML experiment tracker. W&B-like Python API, faster UI, no paywalled features.

**Priorities**
- **P0** — MVP: one person can drop TensorBoard / W&B for their own runs.
- **P1** — v1: what a team needs.
- **P2** — later: where we pull ahead.

**Out of scope for now**
- Framework integrations (Lightning, HF Transformers/Accelerate, Keras, JAX, XGBoost/LightGBM, Optuna, Ray Tune). Instead, the Python API looks like W&B's so migrating is a rename.
- Importers (W&B, MLflow, TensorBoard tfevents) — revisit after v1.
- Artifact uploads — artifacts only reference existing files (see §6).

---

## 1. Tech stack

| Layer | Choice |
|---|---|
| Server | Rust: Axum + Tokio, sqlx, official `clickhouse` crate. One static binary with the web UI embedded (rust-embed). |
| Python SDK | Python API on top of a Rust core (PyO3 + maturin). Batching, compression, retries and the local log run on a Rust background thread. |
| Metadata | PostgreSQL. Configs and summaries as JSONB with GIN indexes. |
| Metrics, system metrics, console logs | ClickHouse. |
| File storage (later) | `object_store` crate: local disk by default, any S3 (SeaweedFS, AWS, R2). |
| SDK → server | Protobuf (prost) over HTTPS POST, zstd-compressed, each batch tagged `(run_id, seq)` for safe retries. |
| Web UI | TypeScript, React 19, Vite SPA, TanStack Router/Query/Table/Virtual, Tailwind + shadcn/ui. No Node server in production. |
| Charts | uPlot (canvas) for line charts; WebGL for large scatter / parallel-coordinates plots. |
| UI ↔ server | REST/JSON with OpenAPI (utoipa) + generated TS client for CRUD; Arrow IPC for metric series; SSE/WebSocket for live updates. |

**Licensing rule:** every dependency must be permissively licensed (MIT, Apache 2.0, BSD, PostgreSQL). No AGPL / SSPL / BSL / TSL defaults. ClickHouse is Apache 2.0; Garage is AGPL, so it isn't a default.

**Repo layout**

```
losscape/
  proto/                  # protobuf schemas: the SDK↔server contract
  crates/
    losscape-core/       # shared types, local log, compression, downsampling
    losscape-server/     # Axum binary, embeds web/dist
    losscape-py/         # PyO3 extension (maturin)
  sdk/python/losscape/   # Python API on top of the extension
  web/                    # Vite + React + TS
  deploy/                 # docker-compose, Helm
  docs/
```

The metric store sits behind a `MetricStore` trait so a zero-dependency "lite" mode (SQLite + Parquet) can be added later.

---

## 2. Performance targets (P0, checked in CI)

Targets to aim for, not measurements.

| What | Target |
|---|---|
| `run.log()` overhead (≤20 scalars) | p99 < 10 µs; no network work during the call |
| `init()` | < 100 ms, even if the server can't be reached |
| `finish()` | Flushes within a configurable timeout (default 30 s); never hangs at exit |
| SDK memory | Fixed cap (e.g. 64 MB queue); overflow goes to disk, nothing is dropped |
| Training slowdown | < 1% of step time |
| Ingest, single node (8 vCPU) | ≥ 1M points/s sustained |
| Logged point → visible on an open chart | p95 < 1 s |
| One chart (50 runs × 1M steps each) | p95 < 150 ms |
| Workspace (50 charts × 100 runs) | Usable in < 2 s; smooth 60 fps zoom, pan, hover |
| Zoom / pan on a chart | Instant visual response; sharper data within p95 < 150 ms |
| Runs table (100k runs), filter or sort on any column | < 300 ms |
| Console log viewer | 1M lines scroll smoothly; search < 500 ms |
| Small-team deployment | Runs on 4 vCPU / 8 GB; app server idles under 200 MB RAM |
| Single-node scale | 10B+ points; 1k runs logging at once |
| Durability | Training process killed: lose at most ~1 s of points. Network or server down: lose nothing. |

---

## 3. Data integrity (P0)

- Values stored exactly as float64. Points are reduced only when reading, never when storing.
- NaN and Inf are kept and shown clearly on charts.
- Idempotent ingest: client retries never create duplicate points.
- Steps may arrive out of order or late (eval metrics logged at earlier steps, backfills). No "step must increase" warnings.
- Both client and server timestamps are stored, so clock differences between machines can be corrected.
- When overloaded, the server slows the SDK down (which buffers to disk) instead of dropping data.
- Clear limits (keys per run, key length, payload size) with readable errors.

---

## 4. Python SDK

### API
- **P0** API that looks like W&B's: `init`, `log`, `config`, `summary`, `finish`, `alert`, `Api().runs(...)`. Works as a context manager and in Jupyter.
- **P1** W&B-compatible layer for the common calls, so `import losscape as wandb` works.
- **P2** CLI and HTTP clients for other languages.

### Run names
- **P0** Every run gets a random readable name by default, e.g. `brave-falcon-42`.
- **P0** The generator is also a public utility: `losscape.utils.random_name(seed=None)`. Pure Python, built-in word lists. Collisions don't matter (run ID is separate).

### Logging
- **P0** Scalar `log()` as fast as possible: in-memory queue, never blocks training (see Design 2).
- **P0** Any metric can be the x-axis (step, epoch, tokens, wall time, relative time).
- **P0** Local write-ahead log, offline mode, `losscape sync` to upload later. Works on air-gapped clusters.
- **P0** Resume a run by ID. Run IDs are generated on the client, so `init` never waits for the server.
- **P0** Safe when the process forks (DataLoader workers) and under multiprocessing.
- **P1** Rich data types: images (with masks and bounding boxes), histograms, tables, audio, video, text, HTML.
- **P1** Distributed training: every rank of a DDP/FSDP/DeepSpeed job can write into one run, or ranks can be grouped.
- **P2** Fork or rewind a run from step N.

### Captured automatically
- **P0** Git commit + diff of uncommitted changes, command line, Python environment (`pip freeze` or `uv.lock`), CUDA version, driver, GPU model, hostname, Slurm job ID, container image.
- **P0** System metrics: NVIDIA GPU usage, memory, power, temperature (NVML); CPU, RAM, disk, network.
- **P0** stdout and stderr, processed so `\r` progress bars display correctly (see Design 3).
- **P0** Values that look like secrets (`*KEY*`, `*TOKEN*`, `*SECRET*`) are hidden in captured environment variables and configs.
- **P1** AMD (ROCm) and Apple Silicon GPU metrics.

### Packaging
- **P0** Wheels for Linux x86_64/aarch64, macOS arm64, Windows; Python 3.10+, including free-threaded builds.

---

## 5. Runs and UI

### Runs
- **P0** Projects, runs, groups (e.g. seeds of the same config), notes.
- **P0** **Tags**: many per run.
- **P0** **Aliases**: a name unique within a project that points to exactly one run (e.g. `baseline`, `best`). Setting it on another run moves it. Usable everywhere: `api.run("proj/@baseline")`, URLs, chart filters.
- **P0** Run states (running, finished, crashed, killed) with crash detection through heartbeats. Crashed runs show exit code and last log lines.
- **P1** Bulk actions: tag, alias, move, archive, delete with a trash bin.

### Overview page, configurable from the UI (P0, main feature)
- A grid of widgets you can drag and resize: runs table, line charts, scatter and bar charts of summary values, parallel coordinates, leaderboard, config diff, markdown.
- **Run sets:** each widget uses either the page's run selection (filter, tags, aliases) or its own. Comparing runs = pick the runs once, every widget follows.
- **Saved views:** multiple layouts per project, each with a shareable URL. Layouts are stored as versioned JSON in Postgres.

### Runs table
- **P0** Any config or summary value as a column; sort, group, pin columns.
- **P0** Filter syntax, e.g. `config.lr < 1e-3 and tag:baseline and state:finished and alias:best`.
- **P0** Virtualized rendering for 100k rows.

### Line charts
- **P0** Every metric's chart is configurable: x-axis, smoothing (EMA, time-weighted EMA, running average), log scale, range, aggregation (mean, min/max band, last).
- **P0** Downsampled when large, with a min/max band so spikes stay visible.
- **P0** Incremental updates: zoom, pan and live data never re-fetch everything (see Design 1).
- **P0** Pick metrics by regex; synced zoom and cursor across charts; tooltips sorted by value.
- **P0** Live updates: new points and new runs appear without a reload.

### Comparing
- **P0** Overlay any set of runs on one chart.
- **P0** Plot metric A against metric B.
- **P0** Side-by-side charts with linked axes.
- **P0** "Relative to baseline" mode: plot run − `@baseline`.
- **P1** Group runs by any key and show mean ± std or min–max bands across seeds.
- **P1** Config diff highlighting only the keys that differ; code diff; environment diff.

### Other views
- **P0** Run page: overview, config and summary, system metrics, logs (searchable, ANSI colors, `\r` handled), files, git info.
- **P0** The whole view state lives in the URL, so a link opens exactly the same view.
- **P0** Dark mode.
- **P1** Workspace with charts created automatically from logged keys, in sections by prefix (`train/`, `val/`).
- **P1** Other chart types: scatter, parallel coordinates, hyperparameter importance, histograms over time, media with a step slider, tables, markdown.
- **P1** Command palette (Cmd+K), keyboard shortcuts, export charts as PNG/SVG/CSV.
- **P1** Mobile layout good enough to check a run from your phone.
- **P2** Custom charts with Vega-Lite.

---

## 6. Artifacts

- **P0 (for now: references only)** An artifact stores a URI (`s3://…`, `gs://…`, `/nfs/…`) plus optional size, hash and modified time. Nothing is uploaded.
- **P0** Versions and aliases (`latest`, `best`, `prod`).
- **P0** Linked to runs as input or output; lineage graph of which run produced or used which artifact.
- **P2** Uploads: content-addressed dedup, resumable multipart uploads straight to storage through signed URLs, files > 100 GB.
- **P2** Retention rules and automatic expiry for checkpoints.
- **P2** Model registry with stages.

---

## 7. Collaboration and alerts

- **P1** Alerts from code with `run.alert()`.
- **P1** Built-in alert rules: NaN loss, run crashed, run stalled for N minutes, metric threshold → Slack, Discord, email, webhook.
- **P1** Reports: rich-text documents with live or frozen embedded charts, comments, read-only share links.
- **P1** Comments and @mentions on runs.

---

## 8. API access and AI

- **P1** Query API returning pandas, polars or Arrow (`api.runs(filter).metrics(...)`), plus Parquet export.
- **P1** MCP server so coding agents can query experiments ("which lr had the best val loss?").
- **P1** Everything the UI can do is available through a versioned public API with an OpenAPI spec.
- **P2** Tracing and evaluation for LLM work: prompts/responses, tokens, latency, cost, side-by-side comparison tables.
- **P2** Read-only SQL console over metrics, limited to the user's projects.
- **P2** Webhooks and automations (e.g. alias set to `prod` → trigger CI).
- **P2** GPU-hours and cost per run and per project.

---

## 9. Sweeps (P2)

- Grid, random and Bayesian search with early stopping (ASHA/Hyperband). Use Optuna as the engine rather than writing our own.
- Agents run on Slurm, Kubernetes or plain machines; the sweep controller resumes after a restart.

---

## 10. Self-hosting and operations

- **P0** One-command `docker compose up`: `losscape` (API + UI), `postgres`, `clickhouse`.
- **P0** No outgoing network calls; telemetry is opt-in only.
- **P0** Configured through environment variables or one TOML file.
- **P0** Database migrations run automatically on startup.
- **P0** SDK–server protocol versioning: older SDKs keep working against newer servers.
- **P0** `/healthz` and `/readyz` endpoints.
- **P1** Helm chart and ARM64 images.
- **P1** Documented backup and restore (Postgres, ClickHouse, file storage).
- **P1** Prometheus `/metrics` and structured logs.
- **P1** Storage usage dashboard and per-project quotas.
- **P2** Stateless app servers that scale horizontally; high availability.
- **P2** Zero-dependency "lite" mode (SQLite + Parquet) to compete with `tensorboard --logdir`.

---

## 11. Security and access

- **P0** Local accounts and per-user API keys. Every query is limited to the caller's projects on the server, not just in the UI.
- **P1** OIDC single sign-on, free.
- **P1** Roles (admin, member, viewer) at org, team and project level; project visibility (private, team, public).
- **P1** Service accounts with scoped keys, rate limiting, audit log.
- **P2** SAML and SCIM.

---

## Appendix: design notes (to be explored)

### Design 1 — Incremental chart fetching (tiles, like a map)

1. **Fixed bucket grid.** Level `k` splits the step axis into buckets of width `2^k`. A tile is 1024 consecutive buckets. Boundaries don't depend on the view, so results are reusable across zooms and users.
2. **Combinable aggregates.** Each bucket stores `min, max, sum, count, first, last`. Two level-`k` buckets combine exactly into one level-`k+1` bucket, so the client can build coarser levels itself and fold live points into existing buckets.
3. **Level from screen width.** For visible range `[a, b]` drawn `W` pixels wide, use the smallest `k` where `(b−a)/2^k ≤ ~2W`; fetch only uncached tiles.
   - Zoom in: fetch a few finer tiles; meanwhile stretch the coarser tile on screen, then sharpen.
   - Zoom out: coarser tiles are usually cached.
   - Pan: fetch only edge tiles.
4. **Finished tiles never change.** Only the last tile of a running run is mutable; all others are cacheable forever (browser + HTTP).
5. **Live updates are pushed.** The server streams new raw points over SSE/WebSocket; the client folds them into the open tile at each cached level. No re-fetch during training.
6. **Small series skip tiling.** Fewer than ~2×W points → send everything once, then only pushed points.
7. **Batched requests.** A workspace fetches all missing tiles (across runs and metrics) in one request, one Arrow response.

Server side: one ClickHouse query per tile, `GROUP BY intDiv(step, 2^k)` within the tile range, cheap thanks to `ORDER BY (project_id, run_id, key, step)`. Add precomputed levels (materialized views, `AggregatingMergeTree`) only if measurements require it.

Open questions:
- Smoothing: EMA on bucket means is an approximation — accept and label it, or compute on the server at full resolution.
- Another metric as x-axis (e.g. tokens) doesn't fit the step grid: bucket by step and map each bucket to its x value, or fall back to raw points.

### Design 2 — Scalar ingestion

```
log() → in-memory channel (µs) → SDK thread: batch every ~100 ms, append to local log
      → POST batch (run_id, seq), many in flight
server: decode → push to live subscribers → buffer
      → flush to ClickHouse every ~250 ms–1 s or 100k rows → confirm every batch in that flush
SDK: delete batches from its local log once confirmed
```

- Durability comes from the SDK's local log; the server needs no log of its own.
- Confirmation after the ClickHouse write (~1 s) costs nothing because training never waits for it. A server crash loses nothing: the SDK resends, and `(run_id, seq)` makes that safe.
- Not using ClickHouse fire-and-forget async inserts: confirmed data could be lost on crash.
- Live viewers get points before the database write.

Metrics table:

```sql
CREATE TABLE metrics (
  project_id UInt64, run_id UInt64,
  key   LowCardinality(String),
  step  UInt64        CODEC(DoubleDelta, ZSTD),
  ts    DateTime64(3) CODEC(DoubleDelta, ZSTD),
  value Float64       CODEC(Gorilla, ZSTD)
) ENGINE = MergeTree ORDER BY (project_id, run_id, key, step);
```

### Design 3 — Console logs with `\r` and progress bars

- **SDK:** keep the current line in memory and apply `\r`, `\n` and a small ANSI subset (`ESC[A` cursor up, `ESC[K` / `ESC[2K` clear line) — enough for tqdm and rich. Send a line on `\n`; send the state of a line still being rewritten at most once per second as a replaceable line. A tqdm bar becomes one updated line, not thousands.
- **Server:** store `(run_id, line_no, text, ts)` in a ClickHouse `ReplacingMergeTree` so an updated line replaces the old one.
- **UI:** virtualized list with an ANSI color parser (not xterm.js, which struggles with millions of lines).
