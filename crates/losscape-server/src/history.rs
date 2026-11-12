//! Every logged point of some metrics of some runs, for downloads and the SDK's query API
//! (the charts read downsampled tiles instead, see `read::series`), and statistics over all
//! of them.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::Query;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::{self, AuthUser, Role, members};
use crate::error::ApiError;
use crate::read::{UiError, parse_run_id, parse_run_ids, stream_query, unknown_project};
use crate::state::AppState;

/// Metrics per request: a chart's, or a run's whole history.
pub const MAX_KEYS: usize = 1000;

#[derive(Deserialize)]
pub struct HistoryParams {
    /// Run ids, repeated (`run=...&run=...`).
    #[serde(default)]
    run: Vec<String>,
    /// Logged metric names, repeated (`key=...&key=...`).
    #[serde(default)]
    key: Vec<String>,
    /// Only steps from `min_step` to `max_step`, both included.
    min_step: Option<u64>,
    max_step: Option<u64>,
}

/// `GET /api/v0/projects/{project}/history?run=&key=`: one CSV row per logged point,
/// `run,run_id,metric,step,time_utc,value`, by run, metric and step. Streamed: it can be
/// millions of rows. Values are as logged (`nan` and `inf` included); `time_utc` is the
/// client's clock.
pub async fn history(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
    Query(params): Query<HistoryParams>,
) -> Result<Response, UiError> {
    let project_id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, project_id, Role::Viewer, unknown_project).await?;
    let runs = parse_run_ids(&params.run)?;
    if runs.is_empty() {
        return Err(ApiError::invalid("run: at least one run id").into());
    }
    let mut keys = params.key;
    keys.sort_unstable();
    keys.dedup();
    if keys.is_empty() || keys.len() > MAX_KEYS {
        return Err(ApiError::invalid(format!("key: 1-{MAX_KEYS} metric names")).into());
    }

    // Names for the CSV, which also tells runs of another project from runs of this one.
    let names: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, name FROM runs WHERE project_id = $1 AND id = ANY($2)")
            .bind(project_id)
            .bind(&runs)
            .fetch_all(state.pg())
            .await?;
    if names.len() < runs.len() {
        return Err(ApiError::unknown_run().into());
    }

    let ids = vec!["toUUID(?)"; names.len()].join(", ");
    let labels = vec!["?"; names.len()].join(", ");
    let key_list = vec!["?"; keys.len()].join(", ");
    // `run_id` is a column and a name for the output: everywhere but the output, the column.
    let sql = format!(
        "SELECT transform(run_id, [{ids}], [{labels}], '') AS run, toString(run_id) AS run_id,
                key AS metric, step, toString(ts, 'UTC') AS time_utc, value
         FROM metrics FINAL
         WHERE project_id = ? AND run_id IN ({ids}) AND key IN ({key_list})
           AND step >= ? AND step <= ?
         ORDER BY run_id, key, step
         SETTINGS prefer_column_name_to_alias = 1"
    );
    let mut query = state.ch_read().query(&sql);
    for (id, _) in &names {
        query = query.bind(id.to_string());
    }
    for (_, name) in &names {
        query = query.bind(name.as_str());
    }
    query = query.bind(project_id as u64);
    for (id, _) in &names {
        query = query.bind(id.to_string());
    }
    for key in &keys {
        query = query.bind(key.as_str());
    }
    query = query
        .bind(params.min_step.unwrap_or(0))
        .bind(params.max_step.unwrap_or(u64::MAX));

    let body = stream_query(query, "CSVWithNames", "history").await?;
    // A download, named by whoever asks for it (the UI names it after the chart).
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment"),
        ],
        body,
    )
        .into_response())
}

/// Metrics per stats request.
pub const MAX_STATS_ITEMS: usize = 500;

#[derive(Deserialize)]
pub struct StatsRequest {
    items: Vec<StatsItem>,
}

#[derive(Deserialize)]
pub struct StatsItem {
    run: String,
    key: String,
}

#[derive(Serialize, clickhouse::Row, Deserialize)]
pub struct MetricStats {
    run: String,
    key: String,
    /// Finite points; the rest is over those (null when there are none).
    n: u64,
    min: Option<f64>,
    max: Option<f64>,
    mean: Option<f64>,
    median: Option<f64>,
}

/// `POST /api/v0/metric-stats` `{"items": [{"run", "key"}]}`: count, min, max, mean and
/// exact median over every logged point of each, for formulas over a whole run. Metrics a
/// run doesn't have are left out.
pub async fn stats(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<StatsRequest>,
) -> Result<Json<Vec<MetricStats>>, UiError> {
    if req.items.is_empty() || req.items.len() > MAX_STATS_ITEMS {
        return Err(ApiError::invalid(format!("1-{MAX_STATS_ITEMS} items per request")).into());
    }
    // Keys per run, each run checked once.
    let mut by_run: BTreeMap<Uuid, Vec<String>> = BTreeMap::new();
    for item in req.items {
        let keys = by_run.entry(parse_run_id(&item.run)?).or_default();
        if !keys.contains(&item.key) {
            keys.push(item.key);
        }
    }
    let mut parts = Vec::with_capacity(by_run.len());
    let mut binds: Vec<(u64, Uuid, Vec<String>)> = Vec::with_capacity(by_run.len());
    for (run_id, keys) in by_run {
        let project_id = auth::require_run(&state, &user, run_id, Role::Viewer).await?;
        let key_list = vec!["?"; keys.len()].join(", ");
        parts.push(format!(
            "SELECT toString(run_id) AS run, key, count() AS n,
                    if(n = 0, NULL, min(value)) AS min, if(n = 0, NULL, max(value)) AS max,
                    if(n = 0, NULL, avg(value)) AS mean,
                    if(n = 0, NULL, medianExact(value)) AS median
             FROM metrics FINAL
             WHERE project_id = ? AND run_id = toUUID(?) AND key IN ({key_list})
               AND isFinite(value)
             GROUP BY run_id, key"
        ));
        binds.push((project_id as u64, run_id, keys));
    }
    let mut query = state.ch_read().query(&format!(
        "{} SETTINGS prefer_column_name_to_alias = 1",
        parts.join(" UNION ALL ")
    ));
    for (project_id, run_id, keys) in binds {
        query = query.bind(project_id).bind(run_id.to_string());
        for key in keys {
            query = query.bind(key);
        }
    }
    Ok(Json(query.fetch_all().await.map_err(ApiError::from)?))
}
