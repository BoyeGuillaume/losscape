//! A project's board (its Overview tab): which runs it shows, as a query, a sort and a limit.
//! Stored as written and applied by the UI, like metric definitions and for the same reason:
//! the server checks shape and size, the UI what a query means.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::types::Json as SqlJson;

use crate::auth::{self, AuthUser, Role, members};
use crate::error::ApiError;
use crate::read::{UiError, unknown_project};
use crate::state::AppState;

const MAX_QUERY: usize = 2_000;
const MAX_SORT: usize = 300;
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
        }
    }
}

/// `GET /api/v0/projects/{project}/board` (members).
pub async fn get(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
) -> Result<Json<Board>, UiError> {
    let id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, id, Role::Viewer, unknown_project).await?;
    let board: Option<SqlJson<Board>> =
        sqlx::query_scalar("SELECT board FROM projects WHERE id = $1")
            .bind(id)
            .fetch_one(state.pg())
            .await?;
    Ok(Json(board.map(|b| b.0).unwrap_or_default()))
}

/// `PUT /api/v0/projects/{project}/board` `{"query", "sort": {"id", "desc"}, "limit"}` (editors).
pub async fn put(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
    Json(board): Json<Board>,
) -> Result<StatusCode, UiError> {
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
    sqlx::query("UPDATE projects SET board = $1 WHERE id = $2")
        .bind(SqlJson(&board))
        .bind(id)
        .execute(state.pg())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
