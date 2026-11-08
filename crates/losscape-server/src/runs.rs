//! Run lifecycle endpoints: init (create or resume), heartbeat, finish.

use axum::extract::State;
use axum::http::StatusCode;
use losscape_core::proto::v0::{
    Ack, Heartbeat, RunEnvironment, RunExitState, RunFinish, RunInit, RunInitResponse,
};
use serde_json::{Value, json};
use sqlx::types::Json;
use uuid::Uuid;

use crate::auth::{self, AuthUser, Role, forbidden};
use crate::error::ApiError;
use crate::ingest::window::SeqWindow;
use crate::ingest::{json_object_or_none, parse_run_id, strip_nul};
use crate::live::LiveEvent;
use crate::proto_body::Proto;
use crate::state::AppState;

const MAX_NAME_LEN: usize = 256;
const MAX_TAGS: usize = 100;
const MAX_TAG_LEN: usize = 128;

pub async fn init(
    State(state): State<AppState>,
    user: AuthUser,
    Proto(mut req): Proto<RunInit>,
) -> Result<Proto<RunInitResponse>, ApiError> {
    strip_nul_from_init(&mut req);
    let run_id = parse_run_id(&req.run_id)?;
    crate::projects::validate_name(&req.project)?;
    let name = match req.name.trim() {
        "" => run_id.simple().to_string()[..8].to_owned(),
        name if name.len() > MAX_NAME_LEN => {
            return Err(ApiError::invalid(format!(
                "name: at most {MAX_NAME_LEN} bytes"
            )));
        }
        name => name.to_owned(),
    };
    validate_tags(&req.tags)?;
    let mut seen = std::collections::HashSet::new();
    let tags: Vec<&String> = req.tags.iter().filter(|t| seen.insert(*t)).collect();
    let config: Value = json_object_or_none(req.config_json, "config_json")?
        .map(|raw| serde_json::from_str(&raw).expect("validated"))
        .unwrap_or_else(|| json!({}));
    let (environment, git_diff) = match req.environment {
        Some(env) => environment_json(env)?,
        None => (json!({}), String::new()),
    };
    let group = Some(req.group.trim()).filter(|g| !g.is_empty());

    let mut tx = state.pg().begin().await?;
    let project_id: i64 = sqlx::query_scalar("SELECT id FROM projects WHERE name = $1")
        .bind(&req.project)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "unknown_project",
                format!(
                    "project \"{}\" does not exist; create it in the web UI first",
                    req.project
                ),
            )
        })?;
    let no_access = || {
        forbidden(
            "this project belongs to other users; ask one of its owners to add you as an editor",
        )
    };
    auth::require_role(&state, &user, project_id, Role::Editor, no_access).await?;

    let created = sqlx::query(
        "INSERT INTO runs (id, project_id, name, group_name, tags, notes, config, environment,
                           client_created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8,
                 CASE WHEN $9 > 0 THEN to_timestamp($9 / 1000.0) END)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(run_id)
    .bind(project_id)
    .bind(&name)
    .bind(group)
    .bind(&tags)
    .bind(&req.notes)
    .bind(Json(&config))
    .bind(Json(&environment))
    .bind(req.client_time_ms)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1;

    if created {
        if !git_diff.is_empty() {
            sqlx::query(
                "INSERT INTO run_files (run_id, name, content) VALUES ($1, 'git.diff', $2)",
            )
            .bind(run_id)
            .bind(git_diff.as_bytes())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        state.remember_run(run_id, project_id);
        return Ok(Proto(RunInitResponse {
            run_id: req.run_id,
            project: req.project,
            name,
            next_seq: 0,
            last_step: None,
            next_console_line: 0,
        }));
    }

    let (existing_project, existing_name, next_seq, pending): (i64, String, i64, Vec<i64>) =
        sqlx::query_as(
            "SELECT project_id, name, next_seq, pending_seqs FROM runs WHERE id = $1 FOR UPDATE",
        )
        .bind(run_id)
        .fetch_one(&mut *tx)
        .await?;
    if !req.resume {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "run_exists",
            "a run with this id already exists; pass resume=true to continue it",
        ));
    }
    if existing_project != project_id {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "run_in_other_project",
            "this run belongs to another project",
        ));
    }
    sqlx::query(
        "UPDATE runs SET state = 'running', heartbeat_at = now(), finished_at = NULL,
                         abort_requested_at = NULL,
                         exit_code = NULL, archived_at = NULL, config = config || $2
         WHERE id = $1",
    )
    .bind(run_id)
    .bind(Json(&config))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    state.remember_run(run_id, project_id);

    Ok(Proto(RunInitResponse {
        run_id: req.run_id,
        project: req.project,
        name: existing_name,
        next_seq: SeqWindow::from_db(next_seq, &pending).resume_seq(),
        last_step: last_step(&state, project_id, run_id).await?,
        next_console_line: next_console_line(&state, project_id, run_id).await?,
    }))
}

pub async fn heartbeat(
    State(state): State<AppState>,
    user: AuthUser,
    Proto(req): Proto<Heartbeat>,
) -> Result<Proto<Ack>, ApiError> {
    let run_id = parse_run_id(&req.run_id)?;
    auth::require_run(&state, &user, run_id, Role::Editor).await?;
    // The flusher already bumps heartbeat_at on every batch, so a run that is logging
    // needs no write here. Writing anyway would make the heartbeat compete with the
    // flusher for the run's row lock: with a synchronous commit stuck behind a busy disk,
    // that lock was held for 14 s and stalled all ingest. A heartbeat is soft state, so
    // it also doesn't wait for the WAL fsync.
    let mut tx = state.pg().begin().await?;
    sqlx::query("SET LOCAL synchronous_commit = off")
        .execute(&mut *tx)
        .await?;
    let abort: Option<bool> = sqlx::query_scalar(
        "WITH touched AS (
             UPDATE runs SET heartbeat_at = now()
             WHERE id = $1 AND heartbeat_at < now() - interval '10 seconds'
             RETURNING id
         )
         SELECT abort_requested_at IS NOT NULL FROM runs WHERE id = $1",
    )
    .bind(run_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    let Some(abort) = abort else {
        return Err(ApiError::unknown_run());
    };
    Ok(Proto(Ack {
        run_id: req.run_id,
        abort,
        ..Default::default()
    }))
}

pub async fn finish(
    State(state): State<AppState>,
    user: AuthUser,
    Proto(req): Proto<RunFinish>,
) -> Result<Proto<Ack>, ApiError> {
    let run_id = parse_run_id(&req.run_id)?;
    auth::require_run(&state, &user, run_id, Role::Editor).await?;
    let run_state = match req.state() {
        RunExitState::Unspecified | RunExitState::Finished => "finished",
        RunExitState::Failed => "failed",
        RunExitState::Killed => "killed",
    };
    let updated = sqlx::query(
        "UPDATE runs SET state = $2, exit_code = $3, last_seq = $4,
                         finished_at = now(), heartbeat_at = now()
         WHERE id = $1",
    )
    .bind(run_id)
    .bind(run_state)
    .bind(req.exit_code)
    .bind(req.last_seq as i64)
    .execute(state.pg())
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(ApiError::unknown_run());
    }
    state.live().publish(LiveEvent::State {
        run_id,
        state: run_state,
    });
    Ok(Proto(Ack {
        run_id: req.run_id,
        seq: req.last_seq,
        duplicate: false,
        abort: false,
    }))
}

#[derive(clickhouse::Row, serde::Deserialize)]
struct StepRange {
    points: u64,
    max_step: u64,
}

async fn last_step(
    state: &AppState,
    project_id: i64,
    run_id: Uuid,
) -> Result<Option<u64>, ApiError> {
    let range: StepRange = state
        .ch()
        .query(
            // System metrics are left out: their step is the unix time.
            "SELECT count() AS points, max(step) AS max_step FROM metrics
             WHERE project_id = ? AND run_id = toUUID(?) AND NOT startsWith(key, 'system/')",
        )
        .bind(project_id as u64)
        .bind(run_id.to_string())
        .fetch_one()
        .await?;
    Ok((range.points > 0).then_some(range.max_step))
}

async fn next_console_line(
    state: &AppState,
    project_id: i64,
    run_id: Uuid,
) -> Result<u64, ApiError> {
    Ok(state
        .ch()
        .query(
            "SELECT if(count() = 0, 0, max(line) + 1) FROM console_lines
             WHERE project_id = ? AND run_id = toUUID(?)",
        )
        .bind(project_id as u64)
        .bind(run_id.to_string())
        .fetch_one::<u64>()
        .await?)
}

/// See [`strip_nul`]: JSON is handled by `json_object_or_none`, the rest here.
fn strip_nul_from_init(req: &mut RunInit) {
    let env = req.environment.as_mut();
    let env_fields = env.into_iter().flat_map(|e| {
        [
            &mut e.hostname,
            &mut e.command,
            &mut e.python_version,
            &mut e.sdk_version,
            &mut e.git_remote,
            &mut e.git_commit,
            &mut e.git_diff,
        ]
    });
    env_fields
        .chain([
            &mut req.project,
            &mut req.name,
            &mut req.group,
            &mut req.notes,
        ])
        .chain(&mut req.tags)
        .for_each(|field| {
            if field.contains('\0') {
                *field = strip_nul(field).into_owned();
            }
        });
}

pub(crate) fn validate_tags(tags: &[String]) -> Result<(), ApiError> {
    if tags.len() > MAX_TAGS {
        return Err(ApiError::invalid(format!("tags: at most {MAX_TAGS}")));
    }
    if tags.iter().any(|t| t.is_empty() || t.len() > MAX_TAG_LEN) {
        return Err(ApiError::invalid(format!(
            "tags: each must be 1-{MAX_TAG_LEN} bytes"
        )));
    }
    Ok(())
}

/// Largest `details_json` accepted, and largest git diff stored.
const MAX_DETAILS_BYTES: usize = 1 << 20;
const MAX_GIT_DIFF_BYTES: usize = 2 << 20;

/// The environment as stored on the run (details merged in), and the git diff, which is
/// kept as a run file instead (it can be large).
fn environment_json(env: RunEnvironment) -> Result<(Value, String), ApiError> {
    let mut out = json!({
        "hostname": env.hostname,
        "command": env.command,
        "python_version": env.python_version,
        "sdk_version": env.sdk_version,
        "git": {
            "remote": env.git_remote,
            "commit": env.git_commit,
            "dirty": env.git_dirty,
            "diff_bytes": env.git_diff.len(),
        },
    });
    if env.details_json.len() > MAX_DETAILS_BYTES {
        return Err(ApiError::invalid("environment.details_json: at most 1 MB"));
    }
    if let Some(raw) = json_object_or_none(env.details_json, "environment.details_json")? {
        let Value::Object(details) = serde_json::from_str(&raw).expect("validated") else {
            unreachable!("validated as an object")
        };
        let target = out.as_object_mut().expect("object");
        for (key, value) in details {
            // The fields above win over details with the same name.
            target.entry(key).or_insert(value);
        }
    }
    let mut diff = env.git_diff;
    if diff.len() > MAX_GIT_DIFF_BYTES {
        let mut end = MAX_GIT_DIFF_BYTES;
        while !diff.is_char_boundary(end) {
            end -= 1;
        }
        diff.truncate(end);
        diff.push_str("\n[truncated]\n");
    }
    Ok((out, diff))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert!(validate_tags(&["baseline".into()]).is_ok());
        assert!(validate_tags(&[String::new()]).is_err());
    }
}
