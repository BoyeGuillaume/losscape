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
| Web UI | Svelte 5 + SvelteKit 3 as a static SPA served by the Rust server, Tailwind 4 + shadcn-svelte. No Node server in production. |
| Charts | LayerChart on canvas: min/max band + line, brush-to-zoom, synced crosshair across charts (`ChartGroup`). Series colors: validated colorblind-safe categorical palette, light and dark steps. |
| UI ↔ server | REST/JSON for CRUD; metric tiles as RowBinary (fixed 56-byte rows, ClickHouse output passed through) for the UI and Arrow for other clients; SSE for live updates. |

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
- **P0 (done)** Git commit + diff of uncommitted changes, command line, Python environment (`pip freeze` or `uv.lock`), CUDA version, driver, GPU model, hostname, Slurm job ID, container image. (Packages come from `*.dist-info` names on `sys.path`; the diff is stored as a run file, up to 2 MB; the container image is known for Apptainer, privileged Podman, or `LOSSCAPE_CONTAINER_IMAGE`.)
- **P0 (done)** System metrics: NVIDIA GPU usage, memory, power, temperature (NVML); CPU, RAM, disk, network. Logged as `system/...` metrics every 5 s (step = unix seconds), kept out of the summary. Network traffic is `system/network/recv` and `sent`, in bits per second; disk traffic `system/disk/read` and `write`, in bytes per second. `system/cpu/process_pct` adds up the process's threads, so it can exceed 100%. The UI shows them with units (%, bytes, bps) from built-in metric definitions (`BUILTIN` in `web/src/lib/metrics/registry.ts`), which a project's own definitions override.
- **P0 (done)** stdout and stderr, processed so `\r` progress bars display correctly (see Design 3). Captured at the file-descriptor level (C extensions, subprocesses), through a pseudo-terminal when the output was one; off on Windows and in Jupyter for now; 100 MB per run. The run's environment records whether capture ran and why not (`console=False`, `LOSSCAPE_CONSOLE`, Windows, notebook, or a start error), so the Logs tab tells "nothing printed" from "not recorded" and shows how to turn it on.
- **P0 (done)** Values that look like secrets (`*KEY*`, `*TOKEN*`, `*SECRET*`) are hidden in captured environment variables and configs (whole words in the name, so `max_tokens` is kept; also strings shaped like known tokens).
- **P1** AMD (ROCm) and Apple Silicon GPU metrics.

### Packaging
- **P0** Wheels for Linux x86_64/aarch64, macOS arm64, Windows; Python 3.10+, including free-threaded builds.

---

## 5. Runs and UI

### Project layout
- **P0 (done)** A project has tabs: Overview (its opening page: see below), Runs (the table, with charts of the picked runs below), Compare (picked runs side by side), Metrics, Tags, Settings.

### Runs
- **P0** Projects, runs, groups (e.g. seeds of the same config), notes.
- **P0 (done)** **Tags**: many per run, set by the SDK or added and removed in the UI (on the run page, or on picked runs). Each project gives its tags a color from a palette (or one derived from the name) and a description; clicking a tag filters the runs by it. The Tags tab lists them with their run counts, colors and descriptions.
- **P0 (partly done)** **Aliases**: a name unique within a project that points to exactly one run (e.g. `baseline`, `best`). Setting it on another run moves it. Usable everywhere: `api.run("proj/@baseline")`, URLs, chart filters. Done: set and remove on the run page, `/p/<project>/r/@best`, `alias:best` in filters. To do: `api.run`.
- **P0 (done)** Abort a running run from its page, or picked runs from the Runs tab (editors): the SDK learns about it from its heartbeat (every 5 s) and interrupts the script like Ctrl-C (also one started in the background, which otherwise ignores interrupts), so what was logged is kept and the run ends as killed. A script still running a minute later is terminated.
- **P0** Run states (running, finished, crashed, killed) with crash detection through heartbeats. Crashed runs show their exit code; their output is on the Logs tab (the Overview doesn't repeat it).
- **P1 (partly done)** Bulk actions: tag, alias, move, archive, delete with a trash bin. Done: tag, archive (the trash bin: hidden, restorable, deleted for good after a per-project number of days, 30 by default). To do: alias, move.

### Overview page (P0 done, main feature)
- **One board per project**, its opening page: a query, a sort and a limit (at most 40 runs) choose the runs, edited on the board itself by editors and saved with the project (`/projects/{project}/board`). From the top:
  - **Figures:** the best run of each important metric, ahead of the baseline or the runner-up by how much.
  - **Leaderboard:** the important metrics and the config values that tell these runs apart; run fields that read the same on every row are left out. The Columns menu replaces that choice by hand, saved with the board. Picked runs go to Compare.
  - **Baseline:** a run picked from the leaderboard stays first and is saved with the board. Others show their change against it (points for percentages, otherwise relative; better follows the metric's direction), and charts show it as a dashed line.
  - **Progress over time:** every matching run (not just the limit) as a dot by when it started, with a line of the best so far. Values default to each metric's summary (best or last, as set on its page); a toggle shows last or best instead, without being saved. At most about 2,000 dots are drawn, always with the record runs and those charted.
  - **Explore:** the same runs against a config value (a scatter for numbers, with a log scale over two orders of magnitude; one column of dots per value for text) or another metric, with the same toggle. **Trajectory** instead draws each charted run's path through two metrics as it trains, from its start to its latest point; metrics logged at different rates are paired by their latest value at each step of the sparser one.
  - **Training curves:** of the first 8 runs or the ones ticked, each keeping its color as rows move.
  - Hovering a run picks it out in the table and every chart; clicking a dot opens it.
  - Code: `web/src/routes/p/[project]/(tabs)/overview/`, `web/src/lib/runs/board.ts`, `web/src/lib/components/app/DotChart.svelte`, `crates/losscape-server/src/board.rs`.
- Known limit: a computed metric's summary is calculated from last values, so "best" on one isn't its true best.
- Run pages show their important metrics as figures above the charts and the overview.

Later, if one board proves too little:
- A grid of widgets you can drag and resize: runs table, line charts, scatter and bar charts of summary values, parallel coordinates, leaderboard, config diff, markdown.
- **Run sets:** each widget uses either the page's run selection (filter, tags, aliases) or its own. Comparing runs = pick the runs once, every widget follows.
- **Saved views:** multiple layouts per project, each with a shareable URL. Layouts are stored as versioned JSON in Postgres.

### Runs table
- **P0 (done)** Any config or summary value as a column; sort, group, pin columns. (Group rows by a run field or config value; group headers show column means.)
- **P0 (done)** Config keys aren't default columns (there can be many); add them from the column picker.
- **P0 (done)** Filter syntax, e.g. `config.lr < 1e-3 and tag:baseline and state:finished and alias:best` (also `or`, `not`, parentheses, `~` regex, bare-word search; `web/src/lib/filter.ts`). The filter bar suggests fields, config keys and their values as you type.
- **P0 (done)** Virtualized rendering for 100k rows (only visible rows are in the DOM; filter in ~250 ms for 20k runs). The run list loads newest first, page by page (the first rows show in ~0.4 s, 20k runs in ~0.9 s), then only fetches runs changed since (`since_ms`, from a `runs.updated_at` kept by triggers).

### Metrics tab (P0, done)
Per project, editors; stored in Postgres and applied by the UI when reading: logged data is never rewritten. Code: `web/src/lib/metrics/`, `web/src/lib/components/app/MetricsList.svelte`, `.../metrics/[...name]/+page.svelte`, `crates/losscape-server/src/metric_defs.rs`.
- **A list of metrics**, logged and computed (ƒ), grouped by prefix: ★ important and hidden are one click; each row shows its settings in one line ("% (0–1) · ↑ higher · best"). Then **families** (below).
- **A page per metric** (`/p/<project>/metrics/<name>`): a live preview (its chart for the latest runs), settings (display name, description, which way is better, summary, unit, range; the formula of a computed metric), the runs that have it or rename it (with undo), and reset or delete. Each setting shows what it inherits ("Auto (↑ higher)").
- **Units** are picked as a kind (number, percent, bytes, bits/s, time, custom suffix), how values were logged (0–1 or 0–100; B, KiB, MiB, GiB; ms, s, min, h; a custom suffix alone or after k, M, B; or another factor) and a precision.
- **Groups:** each prefix (`val/`, `system/`) has a page whose settings apply to every metric under it, field by field, unless the metric sets its own. Built-in defaults describe the system metrics.
- **Important metrics** are charted first (their own section), are the first default columns of the run table, and come first in a run's summary. **Hidden** ones are left out of charts and default columns (still filterable).
- **Renames** (per run, because the same name can mean different things in different runs): from the chart menu on the run page, or for many runs at once from the Metrics tab (picked runs, every run that logged it, or runs matching a filter), with an optional factor (`val/pck-8-fine` × 0.01 shown as `val/pck-8`). Renamed metrics line up with other runs' metric of that name everywhere.
- **Families** (`val/pck-4`, `-8`, `-16`, …, also with a suffix: `pck-4px`, `pck-8px`) are found from the names, logged or given by renames, and can be charted as one: each run's summary value against the number.
- **Computed metrics** are listed with the others (marked ƒ, "Computed metric" makes a blank one and opens its page, where its name and formula are edited, with suggestions for metrics and functions at the caret; removed from the list or from their page, with an undo): formulas over logged metrics (`[metric]`, `+ - * / ^`, `min`, `max`, `abs`, `sqrt`, `log`, `log10`, `exp`), charted bucket by bucket where all inputs exist, and summarized.
- **Best values:** ingest keeps each run's min and max per metric (not `system/`), so "best" summaries cost nothing to read; tables sort best first and mark the best value; charts can show the best value so far.

### Line charts
- **P0 (partly done)** Every metric's chart is configurable: x-axis, smoothing (EMA, time-weighted EMA, running average), log scale, range, aggregation (mean, min/max band, last). Done, for the page and per chart: x-axis (step, wall time, relative time, any metric), EMA weighted by points per bucket, log y, min/max band. To do: other smoothers, range, `last` aggregation.
- **P0** Downsampled when large, with a min/max band so spikes stay visible.
- **P0** Incremental updates: zoom, pan and live data never re-fetch everything (see Design 1).
- **P0 (done)** Pick metrics by regex; synced zoom and cursor across charts; tooltips sorted by value.
- **P0 (done)** Live updates: new points and new runs appear without a reload (one SSE stream per page for all its running runs; the run list is polled).

### Comparing
- **P0 (done)** Overlay any set of runs on one chart (tick runs in the table; up to 40). Picked runs are one selection per project, kept across pages and visits: charted below the table, tagged or archived in bulk, and compared.
- **P0 (done)** Plot metric A against metric B (pairs the two metrics' buckets at the same steps).
- **P0** Side-by-side charts with linked axes.
- **P0** "Relative to baseline" mode: plot run − `@baseline`.
- **P1 (done)** Group runs by any key and show mean ± std or min–max bands across seeds.
- **P0 (done)** Compare tab (`/p/<project>/compare`), on the project's picked runs (at least two), shareable by its link (runs and reference run in the URL; up to 40 runs):
  - Pick runs with a filter (Add, Replace or Remove the runs it matches) and one by one (chips with a remove button); also from the Runs tab's selection bar and "Add to compare" on a run's page.
  - Metrics: one row per run, the important metrics as columns (else the most common ones), the best value marked, and the difference to the reference run (green or red when the metric has a direction).
  - Config diff: one column per run, reference first, only the keys that differ by default (or all, with a key search); values different from the reference highlighted, keys a run doesn't set shown as such.
  - Charts of the picked runs.
- **P1** Code diff; environment diff (git commit, packages).

### Other views
- **P0 (partly done)** Run page: overview, config and summary, system metrics, logs (searchable, ANSI colors, `\r` handled), files, git info. Done except files (artifacts). Logs show as a terminal (light or dark with the page, Cascadia Mono for box drawing, block elements and braille, the 16/256/truecolor ANSI palette), stderr lines marked by a red edge. While a run is going, progress bars the log has moved past are pinned below it, as in a real terminal, along with other lines being rewritten; lines reach viewers about 0.1 s after being printed (pushed on arrival, before they are stored). With Top / Bottom (Follow while running) buttons and a download of the whole log as a file (`/runs/{id}/logs.txt`, plain or with its ANSI codes). The config has its own tab: searchable (keys, values, or dotted paths such as `data.ba` for `data.batch_size`; matching branches only, matches highlighted), as a collapsible tree or as highlighted YAML.
- **P0 (done)** The whole view state lives in the URL, so a link opens exactly the same view (filter, sort, columns, pins, charted runs, chart settings, metric regex, zoom).
- **P0 (done)** Dark mode.
- **P1 (done)** Workspace with charts created automatically from logged keys, in sections by prefix (`train/`, `val/`).
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

- **P0 (done)** Accounts with username + password: Argon2id (OWASP parameters, random salt per hash), constant-response sign-in, rate-limited failures. The first account is administrator.
- **P0 (done)** Browser sessions in an `HttpOnly`, `SameSite=Lax` (`Secure` on HTTPS) cookie, stored hashed; cross-site state-changing requests refused (`Sec-Fetch-Site` / `Origin`).
- **P0 (done)** Per-user API keys for the SDK: shown once, stored as SHA-256, revocable, last-use shown.
- **P0 (done)** Project roles: owner (manages members), editor (logs runs), viewer (reads). Checked on the server for every request; non-members get 404 so ids reveal nothing. A project has exactly one owner (a unique index); ownership moves only by transfer in the project settings (danger zone), never by changing roles or removing the owner. Projects are created explicitly in the UI (`POST /projects`, the creator becomes owner); `runs/init` on an unknown project is refused (404 `unknown_project`), so a typo in `project=` never starts a new project.
- **P0 (done)** Parameterized SQL everywhere; UI output escaped; strict Content-Security-Policy, `nosniff`, no framing.
- **P1 (done)** Sign-in with external providers, free: any OpenID Connect provider (PKCE, verified ID token, nonce) and GitHub; accounts can link providers.
- **P1** Teams / organizations, project visibility (private, team, public); admin user management (disable users, reset passwords).
- **P1** Service accounts with scoped keys (one project, read-only), audit log.
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
- Dedup window per run (`runs.next_seq` + `runs.pending_seqs`), not one row per batch (which would grow ~36k rows per run-hour).
- The flusher's Postgres transaction uses `synchronous_commit = off`. Measured: with ClickHouse merging on the same disk, a single fsync'd commit took up to 18 s. A server crash loses nothing; a Postgres/OS crash can lose ~0.6 s of window and summary updates.
- Every batch carries `confirmed_below` (all lower seqs are acknowledged to the client and never re-sent), so a dedup window that lost updates repairs itself instead of getting stuck.

Measured on the dev machine (20 cores, single disk, Postgres + ClickHouse in Docker), 32 runs × 32 metrics × 20k steps:
- 1.1–2.2M points/s with confirm latency p50 ~165 ms, p99 ~300–500 ms while ClickHouse isn't merging.
- During large background merges (100M+ rows rewritten) ClickHouse inserts stall for up to ~8 s and throughput drops to ~0.45M points/s. No data loss; the SDK absorbs it with its in-flight window and local log, and live charts are pushed before the write.
- SIGKILL of the server mid-load: every point stored exactly once after restart (`make crash-test`).

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

- **SDK:** keep the current line in memory and apply `\r`, `\n` and a small ANSI subset (`ESC[A` cursor up, `ESC[K` / `ESC[2K` clear line) — enough for tqdm and rich. Send a line on `\n`; send the state of a line still being rewritten at most four times per second as a replaceable line, or with a new line of the same stream so that a bar moving down never shows twice. A row just cleared (tqdm clears with spaces) is held back for 0.5 s, since it is usually being redrawn. A tqdm bar becomes one updated line, not thousands.
- **Server:** store `(run_id, line_no, text, ts)` in a ClickHouse `ReplacingMergeTree` so an updated line replaces the old one.
- **UI:** virtualized list with an ANSI color parser (not xterm.js, which struggles with millions of lines).
