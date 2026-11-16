//! Asking for a project, when creating one takes an administrator's approval
//! (`LOSSCAPE_PROJECTS=approval`): users ask, administrators accept (the project is created,
//! owned by who asked) or reject (with why, shown to who asked until they dismiss it).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::error::ApiError;
use crate::projects;
use crate::read::UiError;
use crate::state::AppState;

/// Pending requests one user may have at once.
const MAX_PENDING_PER_USER: i64 = 10;
const MAX_TEXT: usize = 1000;

#[derive(Serialize, sqlx::FromRow)]
pub struct Request {
    id: i64,
    name: String,
    note: String,
    /// `pending` or `rejected`.
    state: String,
    /// Why it was rejected.
    reason: String,
    requested_by: String,
    requested_by_name: Option<String>,
    created_at_ms: i64,
    decided_at_ms: Option<i64>,
}

const COLUMNS: &str = "r.id, r.name, r.note, r.state, r.reason,
    u.username AS requested_by, u.display_name AS requested_by_name,
    (extract(epoch from r.created_at) * 1000)::bigint AS created_at_ms,
    (extract(epoch from r.decided_at) * 1000)::bigint AS decided_at_ms";

fn require_admin(user: &AuthUser) -> Result<(), ApiError> {
    user.require_session()?;
    if !user.is_admin {
        return Err(crate::auth::forbidden("administrators only"));
    }
    Ok(())
}

fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "unknown_request",
        "no such project request",
    )
}

/// Files a request for project `name` by `user` (from `POST /api/v1/projects`).
pub async fn ask(
    state: &AppState,
    user: &AuthUser,
    name: &str,
    note: &str,
) -> Result<(), ApiError> {
    projects::validate_name(name)?;
    if note.len() > MAX_TEXT {
        return Err(ApiError::invalid(format!("note: at most {MAX_TEXT} bytes")));
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM projects WHERE lower(name) = lower($1))")
            .bind(name)
            .fetch_one(state.pg())
            .await?;
    if exists {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "project_exists",
            format!("a project named \"{name}\" already exists"),
        ));
    }
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM project_requests WHERE requested_by = $1 AND state = 'pending'",
    )
    .bind(user.id)
    .fetch_one(state.pg())
    .await?;
    if pending >= MAX_PENDING_PER_USER {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_requests",
            format!(
                "at most {MAX_PENDING_PER_USER} projects asked for at once: wait for an answer"
            ),
        ));
    }
    let inserted = sqlx::query(
        "INSERT INTO project_requests (name, requested_by, note) VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(name)
    .bind(user.id)
    .bind(note.trim())
    .execute(state.pg())
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "already_requested",
            format!(
                "a project named \"{name}\" was already asked for: an administrator will answer"
            ),
        ));
    }
    tracing::info!(project = name, user = user.id, "project asked for");
    Ok(())
}

/// `GET /api/v1/me/project-requests`: the caller's requests, pending or rejected.
pub async fn mine(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<Request>>, UiError> {
    Ok(Json(
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM project_requests r JOIN users u ON u.id = r.requested_by
             WHERE r.requested_by = $1 ORDER BY r.created_at DESC"
        ))
        .bind(user.id)
        .fetch_all(state.pg())
        .await?,
    ))
}

/// `DELETE /api/v1/me/project-requests/{id}`: withdraws a pending request, or dismisses a
/// rejected one.
pub async fn withdraw(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, UiError> {
    let gone = sqlx::query("DELETE FROM project_requests WHERE id = $1 AND requested_by = $2")
        .bind(id)
        .bind(user.id)
        .execute(state.pg())
        .await?;
    if gone.rows_affected() == 0 {
        return Err(not_found().into());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v1/admin/project-requests` (administrators): pending requests, oldest first.
pub async fn pending(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<Request>>, UiError> {
    require_admin(&user)?;
    Ok(Json(
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM project_requests r JOIN users u ON u.id = r.requested_by
             WHERE r.state = 'pending' ORDER BY r.created_at"
        ))
        .fetch_all(state.pg())
        .await?,
    ))
}

/// `POST /api/v1/admin/project-requests/{id}/accept` (administrators): creates the project,
/// owned by who asked.
pub async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, UiError> {
    require_admin(&user)?;
    let found: Option<(String, i64)> = sqlx::query_as(
        "SELECT name, requested_by FROM project_requests WHERE id = $1 AND state = 'pending'",
    )
    .bind(id)
    .fetch_optional(state.pg())
    .await?;
    let (name, owner) = found.ok_or_else(not_found)?;
    projects::insert(&state, &name, owner).await?;
    sqlx::query("DELETE FROM project_requests WHERE id = $1")
        .bind(id)
        .execute(state.pg())
        .await?;
    tracing::info!(
        project = name,
        admin = user.id,
        owner,
        "project request accepted"
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct Reject {
    #[serde(default)]
    reason: String,
}

/// `POST /api/v1/admin/project-requests/{id}/reject` `{"reason"}` (administrators).
pub async fn reject(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(body): Json<Reject>,
) -> Result<StatusCode, UiError> {
    require_admin(&user)?;
    if body.reason.len() > MAX_TEXT {
        return Err(ApiError::invalid(format!("reason: at most {MAX_TEXT} bytes")).into());
    }
    let done = sqlx::query(
        "UPDATE project_requests
         SET state = 'rejected', reason = $2, decided_by = $3, decided_at = now()
         WHERE id = $1 AND state = 'pending'",
    )
    .bind(id)
    .bind(body.reason.trim())
    .bind(user.id)
    .execute(state.pg())
    .await?;
    if done.rows_affected() == 0 {
        return Err(not_found().into());
    }
    tracing::info!(request = id, admin = user.id, "project request rejected");
    Ok(StatusCode::NO_CONTENT)
}
