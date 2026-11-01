//! Read API for the web UI (JSON, plus Arrow for metric series).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::types::Json as SqlJson;
use uuid::Uuid;

use losscape_core::proto::v0::{ConsoleLine, ConsoleStream};

use crate::auth::{self, AuthUser, Role, members};
use crate::error::ApiError;
use crate::state::AppState;

/// Buckets per tile (see docs/REQUIREMENTS.md, Design 1).
pub const TILE_BUCKETS: u64 = 1024;
const MAX_RANGES: usize = 512;
const MAX_TILES_PER_RANGE: u64 = 64;
const MAX_LEVEL: u32 = 63;

/// `ApiError` rendered as JSON for the browser.
#[derive(Debug)]
pub struct UiError(ApiError);

impl IntoResponse for UiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({"code": self.0.code, "message": self.0.message}));
        (self.0.status, body).into_response()
    }
}

impl<E: Into<ApiError>> From<E> for UiError {
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

pub fn parse_run_id(id: &str) -> Result<Uuid, ApiError> {
    id.parse::<Uuid>()
        .ok()
        .filter(|u| u.get_version_num() == 7)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "unknown_run", "not a run id"))
}

/// Run state as shown to users: a running run without a heartbeat for 2 minutes (the SDK
/// sends one every 30 s) is "crashed".
const STATE_SQL: &str =
    "CASE WHEN r.state = 'running' AND r.heartbeat_at < now() - interval '2 minutes'
                         THEN 'crashed' ELSE r.state END";

/// Milliseconds since the epoch, for JSON.
fn ms(column: &str) -> String {
    format!("(extract(epoch from {column}) * 1000)::bigint")
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ProjectSummary {
    name: String,
    /// The caller's role.
    role: String,
    runs: i64,
    last_activity_ms: Option<i64>,
}

/// Projects the caller is a member of (admins see all).
pub async fn projects(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<ProjectSummary>>, UiError> {
    let sql = format!(
        "SELECT p.name, COALESCE(m.role, 'owner') AS role, count(r.id) AS runs, {} AS last_activity_ms
         FROM projects p
         LEFT JOIN project_members m ON m.project_id = p.id AND m.user_id = $1
         LEFT JOIN runs r ON r.project_id = p.id AND r.archived_at IS NULL
         WHERE m.user_id IS NOT NULL OR $2
         GROUP BY p.id, m.role ORDER BY last_activity_ms DESC NULLS LAST, p.name",
        ms("max(r.heartbeat_at)")
    );
    Ok(Json(
        sqlx::query_as(&sql)
            .bind(user.id)
            .bind(user.is_admin)
            .fetch_all(state.pg())
            .await?,
    ))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct RunSummary {
    id: Uuid,
    name: String,
    state: String,
    group_name: Option<String>,
    tags: Vec<String>,
    aliases: Vec<String>,
    created_at_ms: i64,
    /// Start on the client's clock (the clock of metric timestamps), for relative time.
    start_ms: i64,
    /// Archived runs are deleted after the project's `archive_days`.
    archived_at_ms: Option<i64>,
    /// Highest step logged (system metrics aside); null before the first point.
    last_step: Option<i64>,
    heartbeat_at_ms: i64,
    finished_at_ms: Option<i64>,
    summary: SqlJson<Value>,
    config: SqlJson<Value>,
}

/// Columns shared by run listings: aliases, and timestamps as epoch ms.
fn run_columns() -> String {
    format!(
        "COALESCE((SELECT array_agg(a.alias ORDER BY a.alias) FROM run_aliases a WHERE a.run_id = r.id), '{{}}') AS aliases,
         {} AS created_at_ms, {} AS start_ms, {} AS heartbeat_at_ms, {} AS finished_at_ms,
         {} AS archived_at_ms, r.last_step",
        ms("r.created_at"),
        ms("COALESCE(r.client_created_at, r.created_at)"),
        ms("r.heartbeat_at"),
        ms("r.finished_at"),
        ms("r.archived_at"),
    )
}

/// Most recent runs listed per project (the UI filters and sorts them itself).
const MAX_LISTED_RUNS: i64 = 100_000;

#[derive(Deserialize)]
pub struct RunsParams {
    /// true: only archived runs (the others are listed by default).
    #[serde(default)]
    archived: bool,
}

pub async fn project_runs(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
    Query(params): Query<RunsParams>,
) -> Result<Json<Vec<RunSummary>>, UiError> {
    let project_id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, project_id, Role::Viewer, unknown_project).await?;
    let sql = format!(
        "SELECT r.id, r.name, {STATE_SQL} AS state, r.group_name, r.tags, {}, r.summary, r.config
         FROM runs r WHERE r.project_id = $1 AND (r.archived_at IS NOT NULL) = $3
         ORDER BY r.created_at DESC LIMIT $2",
        run_columns()
    );
    let runs: Vec<RunSummary> = sqlx::query_as(&sql)
        .bind(project_id)
        .bind(MAX_LISTED_RUNS)
        .bind(params.archived)
        .fetch_all(state.pg())
        .await?;
    Ok(Json(runs))
}

pub(crate) fn unknown_project() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "unknown_project", "no such project")
}

#[derive(Serialize, sqlx::FromRow)]
pub struct RunDetail {
    id: Uuid,
    project: String,
    name: String,
    state: String,
    group_name: Option<String>,
    tags: Vec<String>,
    aliases: Vec<String>,
    notes: String,
    exit_code: Option<i32>,
    created_at_ms: i64,
    start_ms: i64,
    archived_at_ms: Option<i64>,
    last_step: Option<i64>,
    heartbeat_at_ms: i64,
    finished_at_ms: Option<i64>,
    summary: SqlJson<Value>,
    config: SqlJson<Value>,
    environment: SqlJson<Value>,
    #[sqlx(skip)]
    metrics: Vec<MetricInfo>,
}

#[derive(Serialize, Deserialize, clickhouse::Row)]
pub struct MetricInfo {
    key: String,
    points: u64,
    min_step: u64,
    max_step: u64,
}

pub async fn run_detail(
    State(state): State<AppState>,
    user: AuthUser,
    Path(run): Path<String>,
) -> Result<Json<RunDetail>, UiError> {
    let run_id = parse_run_id(&run)?;
    auth::require_run(&state, &user, run_id, Role::Viewer).await?;
    let sql = format!(
        "SELECT r.id, p.name AS project, r.name, {STATE_SQL} AS state, r.group_name, r.tags, r.notes,
                r.exit_code, {}, r.summary, r.config, r.environment
         FROM runs r JOIN projects p ON p.id = r.project_id WHERE r.id = $1",
        run_columns()
    );
    let mut detail: RunDetail = sqlx::query_as(&sql)
        .bind(run_id)
        .fetch_optional(state.pg())
        .await?
        .ok_or_else(ApiError::unknown_run)?;
    let project_id = state.project_of(run_id).await?;
    detail.metrics = state
        .ch()
        .query(
            "SELECT key, count() AS points, min(step) AS min_step, max(step) AS max_step
             FROM metrics WHERE project_id = ? AND run_id = toUUID(?)
             GROUP BY key ORDER BY key",
        )
        .bind(project_id as u64)
        .bind(run_id.to_string())
        .fetch_all()
        .await
        .map_err(ApiError::from)?;
    Ok(Json(detail))
}

/// Most runs per metrics or live request (the UI shows a few dozen at a time).
pub const MAX_RUNS_PER_REQUEST: usize = 500;

#[derive(Deserialize)]
pub struct RunsRequest {
    runs: Vec<String>,
}

/// Run ids of a request, deduplicated; at most `MAX_RUNS_PER_REQUEST`.
pub fn parse_run_ids(runs: &[String]) -> Result<Vec<Uuid>, ApiError> {
    if runs.len() > MAX_RUNS_PER_REQUEST {
        return Err(ApiError::invalid(format!(
            "at most {MAX_RUNS_PER_REQUEST} runs per request"
        )));
    }
    let mut ids = runs
        .iter()
        .map(|r| parse_run_id(r))
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

#[derive(Serialize, Deserialize, clickhouse::Row)]
pub struct RunMetricInfo {
    run: String,
    key: String,
    points: u64,
    min_step: u64,
    max_step: u64,
}

/// `POST /api/v0/projects/{project}/metrics` `{"runs": [...]}`: the metrics each run
/// logged, with step ranges. Runs of other projects are left out.
pub async fn project_metrics(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
    Json(req): Json<RunsRequest>,
) -> Result<Json<Vec<RunMetricInfo>>, UiError> {
    let project_id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, project_id, Role::Viewer, unknown_project).await?;
    let runs = parse_run_ids(&req.runs)?;
    if runs.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let placeholders = vec!["toUUID(?)"; runs.len()].join(", ");
    let sql = format!(
        "SELECT toString(run_id) AS run, key, count() AS points, min(step) AS min_step,
                max(step) AS max_step
         FROM metrics WHERE project_id = ? AND run_id IN ({placeholders})
         GROUP BY run_id, key ORDER BY run_id, key"
    );
    let mut query = state.ch().query(&sql).bind(project_id as u64);
    for run in runs {
        query = query.bind(run.to_string());
    }
    Ok(Json(query.fetch_all().await.map_err(ApiError::from)?))
}

#[derive(Deserialize)]
pub struct SeriesRequest {
    ranges: Vec<TileRange>,
}

/// Tiles `from_tile..to_tile` of one metric at one level: buckets of `2^level` steps,
/// `TILE_BUCKETS` buckets per tile.
#[derive(Deserialize)]
pub struct TileRange {
    run: String,
    key: String,
    level: u32,
    from_tile: u64,
    to_tile: u64,
}

/// `POST /api/v0/series`: one row per non-empty bucket (Arrow by default, see `format`):
/// `req` (index into `ranges`), `bucket` (bucket index, Float64), `n` (finite points),
/// `v_min`, `v_max`, `v_sum`, `v_first`, `v_last`, `t_mean` (mean timestamp in epoch ms,
/// client clock, for time x-axes). Built by ClickHouse directly.
#[derive(Deserialize)]
pub struct SeriesParams {
    /// `arrow` (default), `rowbinary` (fixed 64-byte little-endian rows, what the web UI
    /// reads), or `json` (one JSON object per line; debugging, tests).
    format: Option<String>,
}

pub async fn series(
    State(state): State<AppState>,
    user: AuthUser,
    Query(params): Query<SeriesParams>,
    Json(req): Json<SeriesRequest>,
) -> Result<Response, UiError> {
    if req.ranges.is_empty() || req.ranges.len() > MAX_RANGES {
        return Err(ApiError::invalid(format!("1-{MAX_RANGES} ranges per request")).into());
    }
    let mut parts = Vec::with_capacity(req.ranges.len());
    let mut binds: Vec<(u64, String, String, u64, u64)> = Vec::with_capacity(req.ranges.len());
    for (i, range) in req.ranges.iter().enumerate() {
        if range.level > MAX_LEVEL
            || range.to_tile <= range.from_tile
            || range.to_tile - range.from_tile > MAX_TILES_PER_RANGE
        {
            return Err(ApiError::invalid(format!(
                "range {i}: level 0-{MAX_LEVEL}, 1-{MAX_TILES_PER_RANGE} tiles"
            ))
            .into());
        }
        let run_id = parse_run_id(&range.run)?;
        let project_id = auth::require_run(&state, &user, run_id, Role::Viewer).await?;
        let steps_per_tile = (TILE_BUCKETS as u128) << range.level;
        let lo = (range.from_tile as u128 * steps_per_tile).min(u64::MAX as u128) as u64;
        let hi = (range.to_tile as u128 * steps_per_tile).min(u64::MAX as u128) as u64;
        // The level is a validated integer, so it can be inlined.
        parts.push(format!(
            "SELECT toUInt32({i}) AS req, toFloat64(bitShiftRight(step, {level})) AS bucket,
                    toUInt32(countIf(isFinite(value))) AS n,
                    minIf(value, isFinite(value)) AS v_min, maxIf(value, isFinite(value)) AS v_max,
                    sumIf(value, isFinite(value)) AS v_sum,
                    argMinIf(value, step, isFinite(value)) AS v_first,
                    argMaxIf(value, step, isFinite(value)) AS v_last,
                    avg(toFloat64(toUnixTimestamp64Milli(ts))) AS t_mean
             FROM metrics
             WHERE project_id = ? AND run_id = toUUID(?) AND key = ? AND step >= ? AND step < ?
             GROUP BY bucket",
            level = range.level
        ));
        binds.push((
            project_id as u64,
            run_id.to_string(),
            range.key.clone(),
            lo,
            hi,
        ));
    }
    let sql = format!(
        "SELECT * FROM ({}) ORDER BY req, bucket",
        parts.join(" UNION ALL ")
    );
    let mut query = state.ch().query(&sql);
    for (project_id, run_id, key, lo, hi) in binds {
        query = query
            .bind(project_id)
            .bind(run_id)
            .bind(key)
            .bind(lo)
            .bind(hi);
    }
    let (format, content_type) = match params.format.as_deref() {
        Some("json") => ("JSONEachRow", "application/x-ndjson"),
        Some("rowbinary") => ("RowBinary", "application/octet-stream"),
        Some("arrow") | None => ("ArrowStream", "application/vnd.apache.arrow.stream"),
        Some(other) => {
            return Err(ApiError::invalid(format!(
                "format {other:?}: use arrow, rowbinary or json"
            ))
            .into());
        }
    };
    let body = query
        .fetch_bytes(format)
        .map_err(ApiError::from)?
        .collect()
        .await
        .map_err(ApiError::from)?;
    Ok(([(header::CONTENT_TYPE, content_type)], body).into_response())
}

/// A console line as the UI gets it (logs endpoint and live `console` events).
#[derive(Serialize)]
pub struct LogLine<'a> {
    line: u64,
    stream: &'static str,
    ts_ms: i64,
    text: std::borrow::Cow<'a, str>,
}

impl<'a> From<&'a ConsoleLine> for LogLine<'a> {
    fn from(l: &'a ConsoleLine) -> Self {
        Self {
            line: l.line,
            stream: stream_name(l.stream as i8),
            ts_ms: l.time_ms,
            text: l.text.as_str().into(),
        }
    }
}

fn stream_name(stream: i8) -> &'static str {
    if stream == ConsoleStream::Stderr as i8 {
        "stderr"
    } else {
        "stdout"
    }
}

#[derive(Deserialize, clickhouse::Row)]
struct ConsoleRow {
    line: u64,
    ts: i64,
    stream: i8,
    text: String,
}

#[derive(Serialize)]
pub struct Logs {
    /// Number of lines (the last line number + 1).
    count: u64,
    lines: Vec<LogLine<'static>>,
}

#[derive(Deserialize)]
pub struct LogsParams {
    /// Lines `from..to`.
    from: Option<u64>,
    to: Option<u64>,
    /// The last `tail` lines.
    tail: Option<u64>,
    /// Lines containing this text (case-insensitive, colors ignored), at most `limit`.
    q: Option<String>,
    limit: Option<u64>,
}

/// Most lines per logs request.
const MAX_LOG_LINES: u64 = 2_000;
/// ANSI escape sequences, removed before searching.
const ANSI_PATTERN: &str = "\x1b\\[[0-9;?]*[A-Za-z]";

/// `GET /api/v0/runs/{run}/logs?from=&to=` | `?tail=` | `?q=&limit=`: console output.
pub async fn run_logs(
    State(state): State<AppState>,
    user: AuthUser,
    Path(run): Path<String>,
    Query(params): Query<LogsParams>,
) -> Result<Json<Logs>, UiError> {
    let run_id = parse_run_id(&run)?;
    let project_id = auth::require_run(&state, &user, run_id, Role::Viewer).await?;
    let count: u64 = state
        .ch()
        .query(
            "SELECT if(count() = 0, 0, max(line) + 1) FROM console_lines
             WHERE project_id = ? AND run_id = toUUID(?)",
        )
        .bind(project_id as u64)
        .bind(run_id.to_string())
        .fetch_one()
        .await
        .map_err(ApiError::from)?;

    let base = "SELECT line, toUnixTimestamp64Milli(ts) AS ts, toInt8(stream) AS stream, text
                FROM console_lines FINAL WHERE project_id = ? AND run_id = toUUID(?)";
    let query = if let Some(q) = params.q.filter(|q| !q.is_empty()) {
        let limit = params.limit.unwrap_or(500).min(MAX_LOG_LINES);
        state
            .ch()
            .query(&format!(
                "{base} AND positionCaseInsensitiveUTF8(replaceRegexpAll(text, ?, ''), ?) > 0
                 ORDER BY line LIMIT {limit}"
            ))
            .bind(project_id as u64)
            .bind(run_id.to_string())
            .bind(ANSI_PATTERN)
            .bind(q)
    } else {
        let (from, to) = match (params.tail, params.from, params.to) {
            (Some(tail), _, _) => (count.saturating_sub(tail.min(MAX_LOG_LINES)), count),
            (None, from, to) => {
                let from = from.unwrap_or(0);
                (
                    from,
                    to.unwrap_or(from + MAX_LOG_LINES).min(from + MAX_LOG_LINES),
                )
            }
        };
        state
            .ch()
            .query(&format!("{base} AND line >= ? AND line < ? ORDER BY line"))
            .bind(project_id as u64)
            .bind(run_id.to_string())
            .bind(from)
            .bind(to)
    };
    let rows: Vec<ConsoleRow> = query.fetch_all().await.map_err(ApiError::from)?;
    Ok(Json(Logs {
        count,
        lines: rows
            .into_iter()
            .map(|r| LogLine {
                line: r.line,
                stream: stream_name(r.stream),
                ts_ms: r.ts,
                text: r.text.into(),
            })
            .collect(),
    }))
}

/// `GET /api/v0/runs/{run}/files/{name}`: a text file captured with the run (`git.diff`).
pub async fn run_file(
    State(state): State<AppState>,
    user: AuthUser,
    Path((run, name)): Path<(String, String)>,
) -> Result<Response, UiError> {
    let run_id = parse_run_id(&run)?;
    auth::require_run(&state, &user, run_id, Role::Viewer).await?;
    let content: Vec<u8> =
        sqlx::query_scalar("SELECT content FROM run_files WHERE run_id = $1 AND name = $2")
            .bind(run_id)
            .bind(&name)
            .fetch_optional(state.pg())
            .await?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "unknown_file", "no such file"))?;
    Ok((
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        content,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_ids_must_be_v7() {
        assert!(parse_run_id("01a18923ac7d70fc8a16cd20a8298365").is_ok());
        assert!(parse_run_id("00000000000000000000000000000000").is_err());
        assert!(parse_run_id("nope").is_err());
    }
}
