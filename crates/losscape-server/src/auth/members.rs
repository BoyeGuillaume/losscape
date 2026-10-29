//! Project membership, managed by project owners.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use super::avatar::AVATAR_URL_SQL;
use super::{AuthUser, Role, require_role};
use crate::error::ApiError;
use crate::read::UiError;
use crate::state::AppState;

fn unknown_project() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "unknown_project", "no such project")
}

pub async fn project_id(state: &AppState, name: &str) -> Result<i64, ApiError> {
    sqlx::query_scalar("SELECT id FROM projects WHERE name = $1")
        .bind(name)
        .fetch_optional(state.pg())
        .await?
        .ok_or_else(unknown_project)
}

#[derive(Serialize)]
pub struct ProjectAccess {
    name: String,
    /// The caller's role.
    role: &'static str,
    members: Vec<Member>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Member {
    username: String,
    display_name: Option<String>,
    avatar_url: Option<String>,
    role: String,
    added_at_ms: i64,
}

/// `GET /api/v0/projects/{project}/members` (any member).
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
) -> Result<Json<ProjectAccess>, UiError> {
    let id = project_id(&state, &project).await?;
    let role = require_role(&state, &user, id, Role::Viewer, unknown_project).await?;
    let sql = format!(
        "SELECT u.username, u.display_name, {AVATAR_URL_SQL} AS avatar_url, m.role, (extract(epoch from m.added_at) * 1000)::bigint AS added_at_ms
         FROM project_members m JOIN users u ON u.id = m.user_id
         LEFT JOIN user_avatars a ON a.user_id = u.id
         WHERE m.project_id = $1
         ORDER BY CASE m.role WHEN 'owner' THEN 0 WHEN 'editor' THEN 1 ELSE 2 END, lower(u.username)"
    );
    let members = sqlx::query_as(&sql).bind(id).fetch_all(state.pg()).await?;
    Ok(Json(ProjectAccess {
        name: project,
        role: role.as_str(),
        members,
    }))
}

#[derive(Deserialize)]
pub struct SetRole {
    role: String,
}

/// `PUT /api/v0/projects/{project}/members/{username}` (owners): adds a member or changes
/// their role.
pub async fn set_role(
    State(state): State<AppState>,
    user: AuthUser,
    Path((project, username)): Path<(String, String)>,
    Json(body): Json<SetRole>,
) -> Result<StatusCode, UiError> {
    let id = project_id(&state, &project).await?;
    require_role(&state, &user, id, Role::Owner, unknown_project).await?;
    let role = Role::parse(&body.role)
        .ok_or_else(|| ApiError::invalid("role: owner, editor or viewer"))?;
    let mut tx = state.pg().begin().await?;
    let member: i64 = sqlx::query_scalar(
        "SELECT id FROM users WHERE lower(username) = lower($1) AND disabled_at IS NULL",
    )
    .bind(&username)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown_user",
            format!("no user named {username:?}"),
        )
    })?;
    sqlx::query(
        "INSERT INTO project_members (project_id, user_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (project_id, user_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(id)
    .bind(member)
    .bind(role.as_str())
    .execute(&mut *tx)
    .await?;
    ensure_owner_left(&mut tx, id).await?;
    tx.commit().await?;
    state.auth_caches().forget_project(id);
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v0/projects/{project}/members/{username}` (owners; anyone may leave).
pub async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path((project, username)): Path<(String, String)>,
) -> Result<StatusCode, UiError> {
    let id = project_id(&state, &project).await?;
    let leaving = username.eq_ignore_ascii_case(&user.username);
    require_role(
        &state,
        &user,
        id,
        if leaving { Role::Viewer } else { Role::Owner },
        unknown_project,
    )
    .await?;
    let mut tx = state.pg().begin().await?;
    let removed = sqlx::query(
        "DELETE FROM project_members m USING users u
         WHERE m.project_id = $1 AND m.user_id = u.id AND lower(u.username) = lower($2)",
    )
    .bind(id)
    .bind(&username)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if removed == 0 {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown_member",
            "not a member of this project",
        )
        .into());
    }
    ensure_owner_left(&mut tx, id).await?;
    tx.commit().await?;
    state.auth_caches().forget_project(id);
    Ok(StatusCode::NO_CONTENT)
}

/// A project always keeps at least one owner (the transaction is rolled back otherwise).
async fn ensure_owner_left(tx: &mut sqlx::PgConnection, project_id: i64) -> Result<(), ApiError> {
    let owners: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM project_members WHERE project_id = $1 AND role = 'owner'",
    )
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await?;
    if owners == 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "last_owner",
            "a project needs at least one owner",
        ));
    }
    Ok(())
}
