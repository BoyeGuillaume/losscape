//! Tags: runs hold them (set by the SDK, or added and removed in the UI); the project keeps
//! how each one shows (a color from the UI's palette, a description).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::auth::{self, AuthUser, Role, members};
use crate::error::ApiError;
use crate::projects::{Changed, MAX_RUNS_PER_ACTION};
use crate::read::{UiError, parse_run_id, unknown_project};
use crate::runs::validate_tags;
use crate::state::AppState;

/// Colors of the UI's tag palette (each has a light and a dark variant there).
pub const COLORS: [&str; 10] = [
    "gray", "red", "orange", "amber", "green", "teal", "blue", "indigo", "purple", "pink",
];
const MAX_DESCRIPTION: usize = 500;

#[derive(Serialize, sqlx::FromRow)]
pub struct TagInfo {
    name: String,
    /// None: the UI picks one from the name.
    color: Option<String>,
    description: String,
    /// Runs with this tag (archived ones aside).
    runs: i64,
}

/// `GET /api/v0/projects/{project}/tags` (members): tags in use or described, most used
/// first.
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
) -> Result<Json<Vec<TagInfo>>, UiError> {
    let id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, id, Role::Viewer, unknown_project).await?;
    Ok(Json(
        sqlx::query_as(
            "WITH used AS (
                 SELECT t AS name, count(*) AS n FROM runs, unnest(tags) AS t
                 WHERE project_id = $1 AND archived_at IS NULL GROUP BY t
             ), meta AS (
                 SELECT name, color, description FROM project_tags WHERE project_id = $1
             )
             SELECT COALESCE(u.name, m.name) AS name, m.color,
                    COALESCE(m.description, '') AS description, COALESCE(u.n, 0) AS runs
             FROM used u FULL JOIN meta m ON m.name = u.name
             ORDER BY runs DESC, name",
        )
        .bind(id)
        .fetch_all(state.pg())
        .await?,
    ))
}

#[derive(Deserialize)]
pub struct TagStyle {
    color: Option<String>,
    #[serde(default)]
    description: String,
}

/// `PUT /api/v0/projects/{project}/tags/{name}` `{"color", "description"}` (editors).
pub async fn describe(
    State(state): State<AppState>,
    user: AuthUser,
    Path((project, name)): Path<(String, String)>,
    Json(body): Json<TagStyle>,
) -> Result<StatusCode, UiError> {
    let id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, id, Role::Editor, unknown_project).await?;
    check_tag(&name)?;
    if let Some(color) = &body.color
        && !COLORS.contains(&color.as_str())
    {
        return Err(ApiError::invalid(format!("color: one of {}", COLORS.join(", "))).into());
    }
    let description = body.description.trim();
    if description.chars().count() > MAX_DESCRIPTION {
        return Err(ApiError::invalid(format!(
            "description: at most {MAX_DESCRIPTION} characters"
        ))
        .into());
    }
    sqlx::query(
        "INSERT INTO project_tags (project_id, name, color, description) VALUES ($1, $2, $3, $4)
         ON CONFLICT (project_id, name)
         DO UPDATE SET color = EXCLUDED.color, description = EXCLUDED.description",
    )
    .bind(id)
    .bind(&name)
    .bind(&body.color)
    .bind(description)
    .execute(state.pg())
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct TagChange {
    runs: Vec<String>,
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
}

/// `POST /api/v0/projects/{project}/runs/tags` `{"runs", "add", "remove"}` (editors): adds
/// and removes tags on these runs, keeping the order of the others.
pub async fn change(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
    Json(body): Json<TagChange>,
) -> Result<Json<Changed>, UiError> {
    let project_id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, project_id, Role::Editor, unknown_project).await?;
    if body.runs.len() > MAX_RUNS_PER_ACTION {
        return Err(
            ApiError::invalid(format!("at most {MAX_RUNS_PER_ACTION} runs at once")).into(),
        );
    }
    validate_tags(&body.add)?;
    validate_tags(&body.remove)?;
    for tag in &body.add {
        check_tag(tag)?;
    }
    let ids = body
        .runs
        .iter()
        .map(|r| parse_run_id(r))
        .collect::<Result<Vec<_>, _>>()?;
    let changed = sqlx::query(
        "UPDATE runs r SET tags = COALESCE((
             SELECT array_agg(t ORDER BY first) FROM (
                 SELECT t, min(i) AS first
                 FROM unnest(r.tags || $3::text[]) WITH ORDINALITY AS u(t, i)
                 WHERE NOT t = ANY($4::text[]) GROUP BY t
             ) kept), '{}')
         WHERE r.project_id = $1 AND r.id = ANY($2)
           AND (NOT r.tags @> $3::text[] OR r.tags && $4::text[])",
    )
    .bind(project_id)
    .bind(&ids)
    .bind(&body.add)
    .bind(&body.remove)
    .execute(state.pg())
    .await?
    .rows_affected();
    Ok(Json(Changed { changed }))
}

/// A tag typed in the UI: trimmed, no control characters (the SDK's own are only limited
/// in length).
fn check_tag(tag: &str) -> Result<(), ApiError> {
    if tag.trim() != tag || tag.is_empty() || tag.len() > 128 || tag.chars().any(char::is_control) {
        return Err(ApiError::invalid(
            "tag: 1-128 bytes, no surrounding spaces or control characters",
        ));
    }
    Ok(())
}
