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
