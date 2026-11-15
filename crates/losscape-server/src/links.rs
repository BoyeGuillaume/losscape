//! Links to checkpoints that read as what they are: `/ckpt/<project>/<run>:<step>`, with
//! `:latest` and `:best` for a run's last and best one, and `/<path>` after it for one file.
//! The run is its name (when no other run of the project has it), `@<alias>`, or its id.
//! `/ckpt/<checkpoint id>` names one checkpoint for good. All go through the server, checked
//! like the API, whatever the storage.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::Response;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{self, AuthUser, Role, members};
use crate::checkpoints;
use crate::error::ApiError;
use crate::read::{UiError, unknown_project};
use crate::retention;
use crate::state::AppState;

/// Left as they are in a link: what never needs encoding in a path segment.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn encode(part: &str) -> String {
    utf8_percent_encode(part, SEGMENT).to_string()
}

/// What a link names.
#[derive(Debug, PartialEq)]
enum Target {
    Id(Uuid),
    Named {
        project: String,
        run: String,
        selector: String,
    },
}

/// A link's path after `/ckpt/`: what it names, and the file path after it (empty: all).
fn parse(raw: &str) -> Result<(Target, String), ApiError> {
    let parts: Vec<String> = raw
        .split('/')
        .map(|p| percent_decode_str(p).decode_utf8_lossy().into_owned())
        .collect();
    let invalid = || {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown_checkpoint",
            "not a checkpoint link: /ckpt/<project>/<run>:<step, latest or best>[/<file>]",
        )
    };
    if parts.len() >= 2
        && let Some((run, selector)) = parts[1].rsplit_once(':')
    {
        if parts[0].is_empty() || run.is_empty() || selector.is_empty() {
            return Err(invalid());
        }
        let target = Target::Named {
            project: parts[0].clone(),
            run: run.to_owned(),
            selector: selector.to_owned(),
        };
        return Ok((target, parts[2..].join("/")));
    }
    let id = parts[0].parse::<Uuid>().map_err(|_| invalid())?;
    Ok((Target::Id(id), parts[1..].join("/")))
}

/// `GET /ckpt/{*link}`: the checkpoint as a tar archive, or one of its files.
pub async fn serve(
    State(state): State<AppState>,
    user: AuthUser,
    uri: Uri,
    request: HeaderMap,
) -> Result<Response, UiError> {
    let raw = uri.path().strip_prefix("/ckpt/").unwrap_or_default();
    let (target, path) = parse(raw)?;
    let (id, fixed) = match target {
        Target::Id(id) => (id, true),
        Target::Named {
            project,
            run,
            selector,
        } => (
            resolve(&state, &user, &project, &run, &selector).await?,
            false,
        ),
    };
    let mut response = if path.is_empty() {
        checkpoints::serve_archive(&state, &user, id, fixed).await?
    } else {
        checkpoints::serve_path(&state, &user, id, &path, &request, fixed).await?
    };
    // Which checkpoint `:latest` or `:best` was, now.
    response.headers_mut().insert(
        "x-losscape-checkpoint",
        id.to_string().parse().expect("uuid"),
    );
    Ok(response)
}

#[derive(Deserialize)]
pub struct RefParams {
    /// `<project>/<run>:<step, latest or best>`, or a whole link.
    r#ref: String,
}

/// `GET /api/v0/checkpoints/resolve?ref=<project>/<run>:<selector>` (viewers): the checkpoint a
/// link names, with its files: for the SDK, given a link.
pub async fn resolve_ref(
    State(state): State<AppState>,
    user: AuthUser,
    Query(params): Query<RefParams>,
) -> Result<Json<checkpoints::Checkpoint>, UiError> {
    let raw = params.r#ref.trim();
    let raw = raw.split_once("/ckpt/").map_or(raw, |(_, rest)| rest);
    let id = match parse(raw.trim_start_matches('/'))?.0 {
        Target::Id(id) => id,
        Target::Named {
            project,
            run,
            selector,
        } => resolve(&state, &user, &project, &run, &selector).await?,
    };
    Ok(Json(checkpoints::detail_of(&state, &user, id).await?))
}

fn not_found(message: String) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "unknown_checkpoint", message)
}

/// The committed checkpoint a name points to now.
pub async fn resolve(
    state: &AppState,
    user: &AuthUser,
    project: &str,
    run: &str,
    selector: &str,
) -> Result<Uuid, ApiError> {
    let project_id = members::project_id(state, project).await?;
    auth::require_role(state, user, project_id, Role::Viewer, unknown_project)
        .await
        .map_err(|e| checkpoints::checkpoint_access(e, Role::Viewer))?;
    let run_id = find_run(state, project_id, project, run).await?;
    let id = match selector {
        "latest" => sqlx::query_scalar(
            "SELECT id FROM checkpoints WHERE run_id = $1 AND state = 'committed'
             ORDER BY step DESC LIMIT 1",
        )
        .bind(run_id)
        .fetch_optional(state.pg())
        .await?
        .ok_or_else(|| not_found(format!("{run} has no checkpoint")))?,
        "best" => {
            let best = retention::load(state, project_id).await?.best.ok_or_else(|| {
                not_found(format!(
                    "{project} has no metric for the best checkpoint: set one in its settings (Checkpoints)"
                ))
            })?;
            let stored = retention::stored(state, run_id).await?;
            retention::best_of(state, project_id, run_id, &best, &stored)
                .await?
                .ok_or_else(|| {
                    not_found(format!(
                        "no checkpoint of {run} has {} logged by its step",
                        best.metric
                    ))
                })?
        }
        step => {
            let step: i64 = step.replace([',', '_'], "").parse().map_err(|_| {
                ApiError::invalid(format!(
                    "{step:?}: a step, `latest` or `best` after the run's name"
                ))
            })?;
            sqlx::query_scalar(
                "SELECT id FROM checkpoints WHERE run_id = $1 AND step = $2 AND state = 'committed'",
            )
            .bind(run_id)
            .bind(step)
            .fetch_optional(state.pg())
            .await?
            .ok_or_else(|| not_found(format!("{run} has no checkpoint at step {step}")))?
        }
    };
    Ok(id)
}

/// A run of a project by `@alias`, id or name: a name only if no other run has it.
async fn find_run(
    state: &AppState,
    project_id: i64,
    project: &str,
    run: &str,
) -> Result<Uuid, ApiError> {
    let unknown = || {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown_run",
            format!("{project} has no run {run}"),
        )
    };
    if let Some(alias) = run.strip_prefix('@') {
        return sqlx::query_scalar(
            "SELECT run_id FROM run_aliases WHERE project_id = $1 AND alias = $2",
        )
        .bind(project_id)
        .bind(alias)
        .fetch_optional(state.pg())
        .await?
        .ok_or_else(unknown);
    }
    if let Ok(id) = run.parse::<Uuid>() {
        let found: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM runs WHERE project_id = $1 AND id = $2")
                .bind(project_id)
                .bind(id)
                .fetch_optional(state.pg())
                .await?;
        if let Some(id) = found {
            return Ok(id);
        }
    }
    let named: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM runs WHERE project_id = $1 AND name = $2 LIMIT 5")
            .bind(project_id)
            .bind(run)
            .fetch_all(state.pg())
            .await?;
    match named.as_slice() {
        [] => Err(unknown()),
        [id] => Ok(*id),
        several => Err(ApiError::new(
            StatusCode::CONFLICT,
            "ambiguous_run",
            format!(
                "several runs of {project} are named {run} ({}): use the run's id, or an alias (@…)",
                several
                    .iter()
                    .map(|id| id.simple().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

/// The start of a run's checkpoint links: `/ckpt/<project>/<run>`, the run named by its name
/// if no other run of the project has it, else by its id.
pub async fn run_link(state: &AppState, run_id: Uuid) -> Result<String, ApiError> {
    let (project, name, same): (String, String, i64) = sqlx::query_as(
        "SELECT p.name, r.name,
                (SELECT count(*) FROM runs o WHERE o.project_id = r.project_id AND o.name = r.name)
         FROM runs r JOIN projects p ON p.id = r.project_id WHERE r.id = $1",
    )
    .bind(run_id)
    .fetch_one(state.pg())
    .await?;
    let run = if same == 1 && !name.is_empty() && name.parse::<Uuid>().is_err() {
        name
    } else {
        run_id.simple().to_string()
    };
    Ok(format!("/ckpt/{}/{}", encode(&project), encode(&run)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(project: &str, run: &str, selector: &str) -> Target {
        Target::Named {
            project: project.into(),
            run: run.into(),
            selector: selector.into(),
        }
    }

    #[test]
    fn links_name_checkpoints() {
        let id = Uuid::now_v7();
        let p = |s: &str| parse(s).unwrap();
        assert_eq!(p(&id.to_string()), (Target::Id(id), String::new()));
        assert_eq!(
            p(&format!("{}/model/w.pt", id.simple())),
            (Target::Id(id), "model/w.pt".into())
        );
        assert_eq!(
            p("mnist/brave-falcon:120"),
            (named("mnist", "brave-falcon", "120"), String::new())
        );
        assert_eq!(
            p("mnist/@best:latest/a%20b/c.pt"),
            (named("mnist", "@best", "latest"), "a b/c.pt".into())
        );
        // A name with a colon or a slash, encoded: the last colon separates the step.
        assert_eq!(
            p("mnist/lr%3A0.1%2Fv2:best"),
            (named("mnist", "lr:0.1/v2", "best"), String::new())
        );
        for bad in ["", "mnist", "mnist/run:", "mnist/:3", "not-an-id/file"] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn names_are_encoded_for_links() {
        assert_eq!(encode("resnet-cifar10"), "resnet-cifar10");
        assert_eq!(encode("lr:0.1/v2 é"), "lr%3A0.1%2Fv2%20%C3%A9");
    }
}
