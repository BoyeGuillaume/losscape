//! Every logged point of some metrics of some runs, for downloads and the SDK's query API
//! (the charts read downsampled tiles instead, see `read::series`).

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::Query;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{self, AuthUser, Role, members};
use crate::error::ApiError;
use crate::read::{UiError, parse_run_ids, stream_query, unknown_project};
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
