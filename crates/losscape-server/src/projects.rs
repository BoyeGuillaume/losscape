//! Projects and run aliases. Projects are created explicitly (web UI); the SDK only logs
//! to existing ones, so a typo in `project=` can't silently start a new project.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::auth::{self, AuthUser, Role, members};
use crate::error::ApiError;
use crate::read::{UiError, parse_run_id, unknown_project};
use crate::state::AppState;

pub fn validate_name(project: &str) -> Result<(), ApiError> {
    let valid = (1..=100).contains(&project.len())
        && project
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(ApiError::invalid(
            "project: 1-100 characters from [A-Za-z0-9._-]",
        ))
    }
}

#[derive(Deserialize)]
pub struct NewProject {
    name: String,
}

#[derive(Serialize)]
pub struct Created {
    name: String,
}

/// `POST /api/v0/projects`: any signed-in user; the creator becomes its owner.
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<NewProject>,
) -> Result<(StatusCode, Json<Created>), UiError> {
    let name = body.name.trim().to_owned();
    insert(&state, &name, user.id).await?;
    Ok((StatusCode::CREATED, Json(Created { name })))
}

/// Creates project `name` owned by user `owner`.
pub async fn insert(state: &AppState, name: &str, owner: i64) -> Result<(), ApiError> {
    validate_name(name)?;
    let mut tx = state.pg().begin().await?;
    let id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO projects (name) VALUES ($1) ON CONFLICT (name) DO NOTHING RETURNING id",
    )
    .bind(name)
    .fetch_optional(&mut *tx)
    .await?;
    let id = id.ok_or_else(|| {
        ApiError::new(
            StatusCode::CONFLICT,
            "project_exists",
            format!("a project named \"{name}\" already exists"),
        )
    })?;
    sqlx::query("INSERT INTO project_members (project_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(id)
        .bind(owner)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Alias names: like project names, at most 64 characters.
pub fn validate_alias(alias: &str) -> Result<(), ApiError> {
    let valid = (1..=64).contains(&alias.len())
        && alias
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(ApiError::invalid(
            "alias: 1-64 characters from [A-Za-z0-9._-]",
        ))
    }
}

#[derive(Deserialize)]
pub struct SetAlias {
    run: String,
}

/// `PUT /api/v0/projects/{project}/aliases/{alias}` `{"run": id}` (editors): points the
/// alias at the run, moving it if another run had it.
pub async fn set_alias(
    State(state): State<AppState>,
    user: AuthUser,
    Path((project, alias)): Path<(String, String)>,
    Json(body): Json<SetAlias>,
) -> Result<StatusCode, UiError> {
    validate_alias(&alias)?;
    let project_id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, project_id, Role::Editor, unknown_project).await?;
    let run_id = parse_run_id(&body.run)?;
    if state.project_of(run_id).await? != project_id {
        return Err(ApiError::unknown_run().into());
    }
    sqlx::query(
        "INSERT INTO run_aliases (project_id, alias, run_id) VALUES ($1, $2, $3)
         ON CONFLICT (project_id, alias) DO UPDATE SET run_id = EXCLUDED.run_id, updated_at = now()",
    )
    .bind(project_id)
    .bind(&alias)
    .bind(run_id)
    .execute(state.pg())
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct AliasTarget {
    run: uuid::Uuid,
}

/// `GET /api/v0/projects/{project}/aliases/{alias}`: the run it points to.
pub async fn get_alias(
    State(state): State<AppState>,
    user: AuthUser,
    Path((project, alias)): Path<(String, String)>,
) -> Result<Json<AliasTarget>, UiError> {
    let project_id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, project_id, Role::Viewer, unknown_project).await?;
    let run =
        sqlx::query_scalar("SELECT run_id FROM run_aliases WHERE project_id = $1 AND alias = $2")
            .bind(project_id)
            .bind(&alias)
            .fetch_optional(state.pg())
            .await?
            .ok_or_else(unknown_alias)?;
    Ok(Json(AliasTarget { run }))
}

fn unknown_alias() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "unknown_alias", "no such alias")
}

/// `DELETE /api/v0/projects/{project}/aliases/{alias}` (editors).
pub async fn remove_alias(
    State(state): State<AppState>,
    user: AuthUser,
    Path((project, alias)): Path<(String, String)>,
) -> Result<StatusCode, UiError> {
    let project_id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, project_id, Role::Editor, unknown_project).await?;
    let deleted = sqlx::query("DELETE FROM run_aliases WHERE project_id = $1 AND alias = $2")
        .bind(project_id)
        .bind(&alias)
        .execute(state.pg())
        .await?
        .rows_affected();
    if deleted == 0 {
        return Err(unknown_alias().into());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Most runs per archive or restore request.
pub(crate) const MAX_RUNS_PER_ACTION: usize = 10_000;

#[derive(Deserialize)]
pub struct RunIds {
    runs: Vec<String>,
}

#[derive(Serialize)]
pub struct Changed {
    /// Runs actually changed (already archived or unknown runs are skipped).
    pub(crate) changed: u64,
}

/// `POST /api/v0/projects/{project}/runs/archive` `{"runs": [...]}` (editors): hides the
/// runs; they are deleted for good after the project's `archive_days`.
pub async fn archive_runs(
    state: State<AppState>,
    user: AuthUser,
    project: Path<String>,
    body: Json<RunIds>,
) -> Result<Json<Changed>, UiError> {
    set_archived(state, user, project, body, true).await
}

/// `POST /api/v0/projects/{project}/runs/restore` `{"runs": [...]}` (editors).
pub async fn restore_runs(
    state: State<AppState>,
    user: AuthUser,
    project: Path<String>,
    body: Json<RunIds>,
) -> Result<Json<Changed>, UiError> {
    set_archived(state, user, project, body, false).await
}

/// `POST /api/v0/projects/{project}/runs/abort` `{"runs": [...]}` (editors): asks these
/// running runs to stop. Their SDK learns about it from its next heartbeat and interrupts
/// the script, so what was logged is kept.
pub async fn abort_runs(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
    Json(body): Json<RunIds>,
) -> Result<Json<Changed>, UiError> {
    let project_id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, project_id, Role::Editor, unknown_project).await?;
    if body.runs.len() > MAX_RUNS_PER_ACTION {
        return Err(
            ApiError::invalid(format!("at most {MAX_RUNS_PER_ACTION} runs at once")).into(),
        );
    }
    let ids = body
        .runs
        .iter()
        .map(|r| parse_run_id(r))
        .collect::<Result<Vec<_>, _>>()?;
    let changed = sqlx::query(
        "UPDATE runs SET abort_requested_at = now()
         WHERE project_id = $1 AND id = ANY($2) AND state = 'running'
           AND abort_requested_at IS NULL",
    )
    .bind(project_id)
    .bind(&ids)
    .execute(state.pg())
    .await?
    .rows_affected();
    Ok(Json(Changed { changed }))
}

async fn set_archived(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
    Json(body): Json<RunIds>,
    archive: bool,
) -> Result<Json<Changed>, UiError> {
    let project_id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, project_id, Role::Editor, unknown_project).await?;
    if body.runs.len() > MAX_RUNS_PER_ACTION {
        return Err(
            ApiError::invalid(format!("at most {MAX_RUNS_PER_ACTION} runs at once")).into(),
        );
    }
    let ids = body
        .runs
        .iter()
        .map(|r| parse_run_id(r))
        .collect::<Result<Vec<_>, _>>()?;
    let changed = sqlx::query(
        "UPDATE runs SET archived_at = CASE WHEN $3 THEN now() END
         WHERE project_id = $1 AND id = ANY($2) AND (archived_at IS NULL) = $3",
    )
    .bind(project_id)
    .bind(&ids)
    .bind(archive)
    .execute(state.pg())
    .await?
    .rows_affected();
    Ok(Json(Changed { changed }))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ProjectInfo {
    name: String,
    /// The caller's role.
    #[sqlx(skip)]
    role: &'static str,
    created_at_ms: i64,
    archive_days: i32,
    runs: i64,
    archived_runs: i64,
}

/// `GET /api/v0/projects/{project}` (members).
pub async fn project_info(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
) -> Result<Json<ProjectInfo>, UiError> {
    let project_id = members::project_id(&state, &project).await?;
    let role = auth::require_role(&state, &user, project_id, Role::Viewer, unknown_project).await?;
    let mut info: ProjectInfo = sqlx::query_as(
        "SELECT p.name, (extract(epoch from p.created_at) * 1000)::bigint AS created_at_ms,
                p.archive_days,
                count(r.id) FILTER (WHERE r.archived_at IS NULL) AS runs,
                count(r.id) FILTER (WHERE r.archived_at IS NOT NULL) AS archived_runs
         FROM projects p LEFT JOIN runs r ON r.project_id = p.id
         WHERE p.id = $1 GROUP BY p.id",
    )
    .bind(project_id)
    .fetch_one(state.pg())
    .await?;
    info.role = role.as_str();
    Ok(Json(info))
}

#[derive(Deserialize)]
pub struct ProjectUpdate {
    archive_days: Option<i32>,
}

/// `PATCH /api/v0/projects/{project}` `{"archive_days": n}` (owners).
pub async fn update_project(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
    Json(body): Json<ProjectUpdate>,
) -> Result<StatusCode, UiError> {
    user.require_session()?;
    let project_id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, project_id, Role::Owner, unknown_project).await?;
    if let Some(days) = body.archive_days {
        if !(1..=crate::purge::MAX_ARCHIVE_DAYS).contains(&days) {
            return Err(ApiError::invalid(format!(
                "archive_days: 1-{}",
                crate::purge::MAX_ARCHIVE_DAYS
            ))
            .into());
        }
        sqlx::query("UPDATE projects SET archive_days = $2 WHERE id = $1")
            .bind(project_id)
            .bind(days)
            .execute(state.pg())
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v0/projects/{project}` (owners): the project, its runs and all their data.
pub async fn delete_project(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
) -> Result<StatusCode, UiError> {
    user.require_session()?;
    let project_id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, project_id, Role::Owner, unknown_project).await?;
    let mut tx = state.pg().begin().await?;
    for sql in [
        "DELETE FROM run_aliases WHERE project_id = $1",
        "DELETE FROM runs WHERE project_id = $1",
        "DELETE FROM projects WHERE id = $1",
    ] {
        sqlx::query(sql).bind(project_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    state.forget_runs(|_, project| *project == project_id);
    // Metrics and console output go in the background (ClickHouse deletes can be slow);
    // nothing can reach them once the runs are gone.
    let ch = state.ch().clone();
    tokio::spawn(async move {
        if let Err(err) = crate::purge::delete_clickhouse(&ch, project_id, None).await {
            tracing::warn!(project_id, %err, "deleting the project's metrics failed");
        }
    });
    tracing::info!(project_id, project, user = user.id, "project deleted");
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_names() {
        assert!(validate_name("mnist-v2.1_test").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name(&"x".repeat(101)).is_err());
        assert!(validate_alias("best").is_ok());
        assert!(validate_alias("@best").is_err());
        assert!(validate_alias(&"x".repeat(65)).is_err());
    }
}
