//! Custom metrics (per project) and metric renames (per run). Both are stored as written
//! and applied by the UI when reading; logged data is never rewritten. The UI validates
//! the meaning of a definition (patterns, formulas); the server checks shape and size.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::types::Json as SqlJson;

use crate::auth::{self, AuthUser, Role, members};
use crate::error::ApiError;
use crate::read::{UiError, parse_run_id, unknown_project};
use crate::state::AppState;

const MAX_DEFINITIONS: usize = 500;
const MAX_NAME: usize = 256;
const MAX_SPEC_BYTES: usize = 16 * 1024;
const KINDS: [&str; 4] = ["describe", "merge", "family", "formula"];

#[derive(Serialize, Deserialize, sqlx::FromRow)]
pub struct Definition {
    /// A metric name, a pattern (`val/pck-{k}`, `train/*`), or the name of a virtual metric.
    name: String,
    kind: String,
    /// Unit, bounds, direction, summary, label, description; sources (merge), expression
    /// (formula), as a JSON object.
    spec: SqlJson<Value>,
}

fn check_name(what: &str, name: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() || name.len() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(ApiError::invalid(format!(
            "{what}: 1-{MAX_NAME} characters, no control characters"
        )));
    }
    Ok(())
}

/// `GET /api/v0/projects/{project}/metric-definitions` (members).
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
) -> Result<Json<Vec<Definition>>, UiError> {
    let id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, id, Role::Viewer, unknown_project).await?;
    Ok(Json(
        sqlx::query_as(
            "SELECT name, kind, spec FROM metric_definitions WHERE project_id = $1 ORDER BY position",
        )
        .bind(id)
        .fetch_all(state.pg())
        .await?,
    ))
}

/// `PUT /api/v0/projects/{project}/metric-definitions` `[...]` (editors): replaces them all,
/// in order (earlier definitions win when several match).
pub async fn replace(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
    Json(definitions): Json<Vec<Definition>>,
) -> Result<StatusCode, UiError> {
    let id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, id, Role::Editor, unknown_project).await?;
    if definitions.len() > MAX_DEFINITIONS {
        return Err(ApiError::invalid(format!("at most {MAX_DEFINITIONS} definitions")).into());
    }
    let mut names = std::collections::HashSet::new();
    for d in &definitions {
        check_name("name", &d.name)?;
        if !names.insert(d.name.as_str()) {
            return Err(ApiError::invalid(format!("{:?} is defined twice", d.name)).into());
        }
        if !KINDS.contains(&d.kind.as_str()) {
            return Err(ApiError::invalid("kind: describe, merge, family or formula").into());
        }
        if !d.spec.0.is_object() || d.spec.0.to_string().len() > MAX_SPEC_BYTES {
            return Err(ApiError::invalid("spec: a JSON object of at most 16 KB").into());
        }
    }
    let mut tx = state.pg().begin().await?;
    sqlx::query("DELETE FROM metric_definitions WHERE project_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for (position, d) in definitions.iter().enumerate() {
        sqlx::query(
            "INSERT INTO metric_definitions (project_id, position, name, kind, spec)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(id)
        .bind(position as i32)
        .bind(&d.name)
        .bind(&d.kind)
        .bind(&d.spec)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v0/projects/{project}/metric-keys` (members): every metric name logged.
pub async fn keys(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
) -> Result<Json<Vec<String>>, UiError> {
    let id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, id, Role::Viewer, unknown_project).await?;
    Ok(Json(
        sqlx::query_scalar(
            "SELECT key FROM project_metric_keys WHERE project_id = $1 ORDER BY key",
        )
        .bind(id)
        .fetch_all(state.pg())
        .await?,
    ))
}

#[derive(Deserialize)]
pub struct Rename {
    to: String,
}

/// `PUT /api/v0/runs/{run}/metric-renames/{from}` `{"to": name}` (editors): shows metric
/// `from` of this run as `to`.
pub async fn rename(
    State(state): State<AppState>,
    user: AuthUser,
    Path((run, from)): Path<(String, String)>,
    Json(body): Json<Rename>,
) -> Result<StatusCode, UiError> {
    let run_id = parse_run_id(&run)?;
    auth::require_run(&state, &user, run_id, Role::Editor).await?;
    check_name("from", &from)?;
    let to = body.to.trim();
    check_name("to", to)?;
    if to == from {
        return Err(ApiError::invalid("the new name is the same").into());
    }
    sqlx::query(
        "INSERT INTO run_metric_renames (run_id, from_key, to_key) VALUES ($1, $2, $3)
         ON CONFLICT (run_id, from_key) DO UPDATE SET to_key = EXCLUDED.to_key",
    )
    .bind(run_id)
    .bind(&from)
    .bind(to)
    .execute(state.pg())
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v0/runs/{run}/metric-renames/{from}` (editors): back to the logged name.
pub async fn unrename(
    State(state): State<AppState>,
    user: AuthUser,
    Path((run, from)): Path<(String, String)>,
) -> Result<StatusCode, UiError> {
    let run_id = parse_run_id(&run)?;
    auth::require_run(&state, &user, run_id, Role::Editor).await?;
    let deleted = sqlx::query("DELETE FROM run_metric_renames WHERE run_id = $1 AND from_key = $2")
        .bind(run_id)
        .bind(&from)
        .execute(state.pg())
        .await?
        .rows_affected();
    if deleted == 0 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "unknown_rename", "not renamed").into());
    }
    Ok(StatusCode::NO_CONTENT)
}
