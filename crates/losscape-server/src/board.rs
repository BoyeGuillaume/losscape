//! A project's board (its Overview tab): which runs it shows, as a query, a sort and a limit.
//! Stored as written and applied by the UI, like metric definitions and for the same reason:
//! the server checks shape and size, the UI what a query means.

use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use sqlx::types::Json as SqlJson;

use crate::auth::{self, AuthUser, Role, members};
use crate::error::ApiError;
use crate::read::{UiError, unknown_project};
use crate::state::AppState;

const MAX_QUERY: usize = 2_000;
const MAX_SORT: usize = 300;
const MAX_COLUMNS: usize = 100;
/// As many as a chart or a comparison can take.
const MAX_LIMIT: u32 = 40;

#[derive(Serialize, Deserialize)]
pub struct Sort {
    /// A column of the runs table: `created`, `summary.val/acc`, ...
    id: String,
    desc: bool,
}

#[derive(Serialize, Deserialize)]
pub struct Board {
    /// A runs filter; empty: every run.
    query: String,
    sort: Sort,
    limit: u32,
    /// The run others are measured against.
    #[serde(default)]
    baseline: Option<String>,
    /// The config key metrics are plotted against.
    #[serde(default)]
    explore: Option<String>,
    /// The leaderboard's columns, if chosen by hand.
    #[serde(default)]
    columns: Option<Vec<String>>,
}

impl Default for Board {
    fn default() -> Self {
        Self {
            query: String::new(),
            sort: Sort {
                id: "created".to_owned(),
                desc: true,
            },
            limit: 10,
            baseline: None,
            explore: None,
            columns: None,
        }
    }
}

/// A board, as a version of it: it's saved all at once.
#[derive(Serialize, Deserialize)]
pub struct Versioned {
    version: i64,
    #[serde(flatten)]
    board: Board,
}

#[derive(Serialize)]
pub struct Saved {
    version: i64,
}

/// `GET /api/v0/projects/{project}/board` (members).
pub async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
) -> Result<Json<Versioned>, UiError> {
    let id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, id, Role::Viewer, unknown_project).await?;
    let (board, version): (Option<SqlJson<Board>>, i64) =
        sqlx::query_as("SELECT board, board_version FROM projects WHERE id = $1")
            .bind(id)
            .fetch_one(state.pg())
            .await?;
    Ok(Json(Versioned {
        version,
        board: board.map(|b| b.0).unwrap_or_default(),
    }))
}

/// `PUT /api/v0/projects/{project}/board` `{"version", "query", "sort": {"id", "desc"}, "limit",
/// "baseline", "explore", "columns"}` (editors). Refused if `version` isn't the latest.
pub async fn put(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
    Json(Versioned { version, board }): Json<Versioned>,
) -> Result<Json<Saved>, UiError> {
    let id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, id, Role::Editor, unknown_project).await?;
    if board.query.len() > MAX_QUERY {
        return Err(ApiError::invalid(format!("query: at most {MAX_QUERY} bytes")).into());
    }
    if board.sort.id.is_empty()
        || board.sort.id.len() > MAX_SORT
        || board.sort.id.chars().any(char::is_control)
    {
        return Err(ApiError::invalid(format!("sort: 1-{MAX_SORT} bytes")).into());
    }
    if !(1..=MAX_LIMIT).contains(&board.limit) {
        return Err(ApiError::invalid(format!("limit: 1-{MAX_LIMIT}")).into());
    }
    let columns = board.columns.as_deref().unwrap_or_default();
    if columns.len() > MAX_COLUMNS {
        return Err(ApiError::invalid(format!("columns: at most {MAX_COLUMNS}")).into());
    }
    for field in [&board.baseline, &board.explore]
        .into_iter()
        .flatten()
        .chain(columns)
    {
        if field.len() > MAX_SORT || field.chars().any(char::is_control) {
            return Err(ApiError::invalid(format!(
                "baseline, explore, columns: at most {MAX_SORT} bytes"
            ))
            .into());
        }
    }
    let saved: Option<i64> = sqlx::query_scalar(
        "UPDATE projects SET board = $1, board_version = board_version + 1
         WHERE id = $2 AND board_version = $3
         RETURNING board_version",
    )
    .bind(SqlJson(&board))
    .bind(id)
    .bind(version)
    .fetch_optional(state.pg())
    .await?;
    let Some(saved) = saved else {
        return Err(ApiError::stale("the board was changed since it was read").into());
    };
    Ok(Json(Saved { version: saved }))
}
