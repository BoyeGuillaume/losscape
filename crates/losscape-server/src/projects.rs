//! Project creation. Projects are created explicitly (web UI); the SDK only logs to
//! existing ones, so a typo in `project=` can't silently start a new project.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::error::ApiError;
use crate::read::UiError;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_names() {
        assert!(validate_name("mnist-v2.1_test").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name(&"x".repeat(101)).is_err());
    }
}
