//! Read API for the web UI (JSON, plus Arrow for metric series).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::types::Json as SqlJson;
use uuid::Uuid;

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
         LEFT JOIN runs r ON r.project_id = p.id
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
    created_at_ms: i64,
    heartbeat_at_ms: i64,
    finished_at_ms: Option<i64>,
    summary: SqlJson<Value>,
    config: SqlJson<Value>,
}

pub async fn project_runs(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
) -> Result<Json<Vec<RunSummary>>, UiError> {
    let project_id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, project_id, Role::Viewer, unknown_project).await?;
    let sql = format!(
        "SELECT r.id, r.name, {STATE_SQL} AS state, r.group_name, r.tags,
                {} AS created_at_ms, {} AS heartbeat_at_ms, {} AS finished_at_ms, r.summary, r.config
         FROM runs r JOIN projects p ON p.id = r.project_id
         WHERE r.project_id = $1 ORDER BY r.created_at DESC LIMIT 1000",
        ms("r.created_at"),
        ms("r.heartbeat_at"),
        ms("r.finished_at"),
    );
    let runs: Vec<RunSummary> = sqlx::query_as(&sql)
        .bind(project_id)
        .fetch_all(state.pg())
        .await?;
    Ok(Json(runs))
}

fn unknown_project() -> ApiError {
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
    notes: String,
    exit_code: Option<i32>,
    created_at_ms: i64,
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
                r.exit_code, {} AS created_at_ms, {} AS heartbeat_at_ms, {} AS finished_at_ms,
                r.summary, r.config, r.environment
         FROM runs r JOIN projects p ON p.id = r.project_id WHERE r.id = $1",
        ms("r.created_at"),
        ms("r.heartbeat_at"),
        ms("r.finished_at"),
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
/// `v_min`, `v_max`, `v_sum`, `v_first`, `v_last`. Built by ClickHouse directly.
#[derive(Deserialize)]
pub struct SeriesParams {
    /// `arrow` (default), `rowbinary` (fixed 56-byte little-endian rows, what the web UI
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
                    argMaxIf(value, step, isFinite(value)) AS v_last
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
