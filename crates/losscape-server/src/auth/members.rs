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

/// The owner's role never changes here: ownership moves only by [`transfer`].
fn owner_is_fixed(message: &str) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "owner_fixed", message)
}

/// The member who owns the project (exactly one; enforced by a unique index).
async fn owner_of(tx: &mut sqlx::PgConnection, project_id: i64) -> Result<Option<i64>, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT user_id FROM project_members WHERE project_id = $1 AND role = 'owner'",
    )
    .bind(project_id)
    .fetch_optional(&mut *tx)
    .await?)
}

/// `PUT /api/v0/projects/{project}/members/{username}` (owners): adds a member or changes
/// their role, as editor or viewer.
pub async fn set_role(
    State(state): State<AppState>,
    user: AuthUser,
    Path((project, username)): Path<(String, String)>,
    Json(body): Json<SetRole>,
) -> Result<StatusCode, UiError> {
    let id = project_id(&state, &project).await?;
    require_role(&state, &user, id, Role::Owner, unknown_project).await?;
    let role = Role::parse(&body.role)
        .filter(|r| *r != Role::Owner)
        .ok_or_else(|| {
            ApiError::invalid(
                "role: editor or viewer (a project has one owner; transfer ownership in the project settings)",
            )
        })?;
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
    if owner_of(&mut tx, id).await? == Some(member) {
        return Err(owner_is_fixed(
            "the owner's role changes only by transferring ownership (project settings, danger zone)",
        )
        .into());
    }
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
    let owner: Option<String> = sqlx::query_scalar(
        "SELECT u.username FROM project_members m JOIN users u ON u.id = m.user_id
         WHERE m.project_id = $1 AND m.role = 'owner'",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    if owner.is_some_and(|o| o.eq_ignore_ascii_case(&username)) {
        return Err(owner_is_fixed(if leaving {
            "transfer ownership to another member before leaving"
        } else {
            "the owner can't be removed; only they can transfer ownership"
        })
        .into());
    }
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

#[derive(Deserialize)]
pub struct Transfer {
    /// The member who becomes owner.
    username: String,
    /// What the current owner becomes: "editor", "viewer", or "leave" (no longer a member).
    then: String,
}

/// `POST /api/v0/projects/{project}/transfer` (owners): makes another member owner and
/// steps the current owner down, in one transaction (also when an admin does it).
pub async fn transfer(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
    Json(body): Json<Transfer>,
) -> Result<StatusCode, UiError> {
    let id = project_id(&state, &project).await?;
    require_role(&state, &user, id, Role::Owner, unknown_project).await?;
    let then = match body.then.as_str() {
        "leave" => None,
        role => Some(
            Role::parse(role)
                .filter(|r| *r != Role::Owner)
                .ok_or_else(|| ApiError::invalid("then: editor, viewer or leave"))?,
        ),
    };
    let mut tx = state.pg().begin().await?;
    let target: Option<i64> = sqlx::query_scalar(
        "SELECT u.id FROM project_members m JOIN users u ON u.id = m.user_id
         WHERE m.project_id = $1 AND lower(u.username) = lower($2) AND u.disabled_at IS NULL",
    )
    .bind(id)
    .bind(&body.username)
    .fetch_optional(&mut *tx)
    .await?;
    let target = target.ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown_member",
            format!(
                "{} is not a member of this project; add them first",
                body.username
            ),
        )
    })?;
    let owner = owner_of(&mut tx, id).await?;
    if owner == Some(target) {
        return Err(
            ApiError::invalid(format!("{} already owns this project", body.username)).into(),
        );
    }
    // The old owner steps down first: there is only ever one owner.
    if let Some(owner) = owner {
        let query = match then {
            Some(role) => sqlx::query(
                "UPDATE project_members SET role = $3 WHERE project_id = $1 AND user_id = $2",
            )
            .bind(id)
            .bind(owner)
            .bind(role.as_str()),
            None => {
                sqlx::query("DELETE FROM project_members WHERE project_id = $1 AND user_id = $2")
                    .bind(id)
                    .bind(owner)
            }
        };
        query.execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE project_members SET role = 'owner' WHERE project_id = $1 AND user_id = $2")
        .bind(id)
        .bind(target)
        .execute(&mut *tx)
        .await?;
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
