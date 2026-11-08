//! Deleting data for good: runs archived longer than their project's `archive_days`
//! (checked every hour), and everything of a deleted project.

use std::time::Duration;

use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

pub const MAX_ARCHIVE_DAYS: i32 = 3650;
const EVERY: Duration = Duration::from_secs(3600);
/// Runs deleted per round, so one round stays short.
const BATCH: i64 = 500;

/// Checks every hour, for as long as the server runs.
pub fn spawn_purger(state: AppState) {
    tokio::spawn(async move {
        loop {
            match purge_archived(&state).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(runs = n, "deleted archived runs"),
                Err(err) => tracing::warn!(%err.message, "deleting archived runs failed"),
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

/// Deletes runs archived longer than their project keeps them. Returns how many.
pub async fn purge_archived(state: &AppState) -> Result<u64, ApiError> {
    let mut total = 0;
    loop {
        let runs: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT r.id, r.project_id FROM runs r JOIN projects p ON p.id = r.project_id
             WHERE r.archived_at < now() - make_interval(days => p.archive_days) LIMIT $1",
        )
        .bind(BATCH)
        .fetch_all(state.pg())
        .await?;
        if runs.is_empty() {
            return Ok(total);
        }
        // Metrics first: if this stops halfway, the runs are still there to retry.
        let mut by_project: std::collections::HashMap<i64, Vec<Uuid>> = Default::default();
        for &(run, project) in &runs {
            by_project.entry(project).or_default().push(run);
        }
        for (project, ids) in &by_project {
            delete_clickhouse(state.ch(), *project, Some(ids)).await?;
        }
        let ids: Vec<Uuid> = runs.iter().map(|r| r.0).collect();
        sqlx::query("DELETE FROM runs WHERE id = ANY($1)")
            .bind(&ids)
            .execute(state.pg())
            .await?;
        state.forget_runs(|run, _| ids.contains(run));
        total += runs.len() as u64;
    }
}

/// Deletes the metrics and console output of `runs` (all runs if None) of a project.
pub async fn delete_clickhouse(
    ch: &clickhouse::Client,
    project_id: i64,
    runs: Option<&[Uuid]>,
) -> clickhouse::error::Result<()> {
    for table in ["metrics", "console_lines"] {
        let mut sql = format!("DELETE FROM {table} WHERE project_id = ?");
        if let Some(runs) = runs {
            let placeholders = vec!["toUUID(?)"; runs.len()].join(", ");
            sql.push_str(&format!(" AND run_id IN ({placeholders})"));
        }
        let mut query = ch.query(&sql).bind(project_id as u64);
        for run in runs.unwrap_or_default() {
            query = query.bind(run.to_string());
        }
        query.execute().await?;
    }
    Ok(())
}

/// Fills `runs.last_step` for runs logged before it existed, from ClickHouse. Runs once
/// at startup; runs without metrics are looked at again next time (cheap).
pub async fn backfill_last_step(state: &AppState) -> Result<u64, ApiError> {
    #[derive(serde::Deserialize, clickhouse::Row)]
    struct Last {
        run: String,
        last_step: u64,
    }
    let mut filled = 0;
    let mut after = Uuid::nil();
    loop {
        let runs: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT id, project_id FROM runs WHERE last_step IS NULL AND id > $1 ORDER BY id LIMIT $2",
        )
        .bind(after)
        .bind(BATCH)
        .fetch_all(state.pg())
        .await?;
        let Some(&(last, _)) = runs.last() else {
            return Ok(filled);
        };
        after = last;
        let placeholders = vec!["toUUID(?)"; runs.len()].join(", ");
        let mut query = state.ch().query(&format!(
            "SELECT toString(run_id) AS run, max(step) AS last_step FROM metrics
             WHERE run_id IN ({placeholders}) AND NOT startsWith(key, 'system/')
             GROUP BY run_id"
        ));
        for (run, _) in &runs {
            query = query.bind(run.to_string());
        }
        for row in query.fetch_all::<Last>().await.map_err(ApiError::from)? {
            let Ok(run) = row.run.parse::<Uuid>() else {
                continue;
            };
            filled +=
                sqlx::query("UPDATE runs SET last_step = GREATEST(last_step, $2) WHERE id = $1")
                    .bind(run)
                    .bind(row.last_step as i64)
                    .execute(state.pg())
                    .await?
                    .rows_affected();
        }
    }
}

type JsonMap = serde_json::Map<String, serde_json::Value>;

/// Fills what ingest now keeps for runs logged before it did: per-metric min and max
/// (`runs.metric_min`/`metric_max`) and the metric names of each project. Runs once at
/// startup.
pub async fn backfill_metric_index(state: &AppState) -> Result<u64, ApiError> {
    #[derive(serde::Deserialize, clickhouse::Row)]
    struct ProjectKey {
        project_id: u64,
        key: String,
    }
    #[derive(serde::Deserialize, clickhouse::Row)]
    struct Extremes {
        run: String,
        key: String,
        lo: f64,
        hi: f64,
    }
    let indexed: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM project_metric_keys)")
        .fetch_one(state.pg())
        .await?;
    if !indexed {
        let keys: Vec<ProjectKey> = state
            .ch()
            .query("SELECT DISTINCT project_id, key FROM metrics")
            .fetch_all()
            .await
            .map_err(ApiError::from)?;
        let (projects, keys): (Vec<i64>, Vec<String>) = keys
            .into_iter()
            .map(|k| (k.project_id as i64, k.key))
            .unzip();
        // Keys of deleted projects are skipped by the join.
        sqlx::query(
            "INSERT INTO project_metric_keys (project_id, key)
             SELECT k.p, k.key FROM unnest($1::bigint[], $2::text[]) AS k(p, key)
             JOIN projects ON projects.id = k.p ON CONFLICT DO NOTHING",
        )
        .bind(projects)
        .bind(keys)
        .execute(state.pg())
        .await?;
    }

    let mut filled = 0;
    let mut after = Uuid::nil();
    loop {
        let runs: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM runs WHERE metric_min = '{}' AND last_step IS NOT NULL AND id > $1
             ORDER BY id LIMIT $2",
        )
        .bind(after)
        .bind(BATCH)
        .fetch_all(state.pg())
        .await?;
        let Some(&last) = runs.last() else {
            return Ok(filled);
        };
        after = last;
        let placeholders = vec!["toUUID(?)"; runs.len()].join(", ");
        let mut query = state.ch().query(&format!(
            "SELECT toString(run_id) AS run, key, min(value) AS lo, max(value) AS hi FROM metrics
             WHERE run_id IN ({placeholders}) AND isFinite(value) AND NOT startsWith(key, 'system/')
             GROUP BY run_id, key"
        ));
        for run in &runs {
            query = query.bind(run.to_string());
        }
        let rows: Vec<Extremes> = query.fetch_all().await.map_err(ApiError::from)?;
        let mut per_run: std::collections::HashMap<Uuid, (JsonMap, JsonMap)> = Default::default();
        for r in rows {
            let Ok(run) = r.run.parse::<Uuid>() else {
                continue;
            };
            let entry = per_run.entry(run).or_default();
            entry.0.insert(r.key.clone(), r.lo.into());
            entry.1.insert(r.key, r.hi.into());
        }
        for (run, (lo, hi)) in per_run {
            filled += sqlx::query("UPDATE runs SET metric_min = $2, metric_max = $3 WHERE id = $1")
                .bind(run)
                .bind(sqlx::types::Json(lo))
                .bind(sqlx::types::Json(hi))
                .execute(state.pg())
                .await?
                .rows_affected();
        }
    }
}
