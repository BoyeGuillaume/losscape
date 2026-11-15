//! How long checkpoints are kept (per project), and metric values at their steps.
//!
//! By default a project keeps checkpoints until they're deleted by hand. With `days` set, a
//! checkpoint goes that long after its upload, unless it's protected: marked important, its
//! run's last one (`keep_last`), or its run's best one by a metric (`best`): the one where the
//! metric's latest value at its step is lowest or highest (the first of them, on a tie).

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

/// Longest retention, as for archived runs.
pub const MAX_DAYS: i32 = 3650;
const MAX_METRIC_NAME: usize = 256;
/// Checkpoints looked at per pass of the cleaner.
const BATCH: i64 = 1000;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Goal {
    Min,
    Max,
}

impl Goal {
    fn parse(s: &str) -> Self {
        if s == "min" { Self::Min } else { Self::Max }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Min => "min",
            Self::Max => "max",
        }
    }

    /// Whether `a` is better than `b`.
    fn better(self, a: f64, b: f64) -> bool {
        match self {
            Self::Min => a < b,
            Self::Max => a > b,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Best {
    pub metric: String,
    pub goal: Goal,
}

/// A project's retention, as the API shows and takes it.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Retention {
    /// Days after upload; none: kept until deleted.
    pub days: Option<i32>,
    pub keep_last: bool,
    /// Keep each run's best checkpoint (by `best`).
    #[serde(default)]
    pub keep_best: bool,
    /// What makes a run's checkpoint its best: for the `:best` link, the mark in the UI, and
    /// `keep_best`.
    pub best: Option<Best>,
}

impl Retention {
    pub fn check(&self) -> Result<(), ApiError> {
        if let Some(days) = self.days
            && !(1..=MAX_DAYS).contains(&days)
        {
            return Err(ApiError::invalid(format!("days: 1-{MAX_DAYS}")));
        }
        if let Some(best) = &self.best
            && (best.metric.trim().is_empty() || best.metric.len() > MAX_METRIC_NAME)
        {
            return Err(ApiError::invalid(format!(
                "best.metric: a metric name, at most {MAX_METRIC_NAME} bytes"
            )));
        }
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct RetentionRow {
    checkpoint_days: Option<i32>,
    checkpoint_keep_last: bool,
    checkpoint_keep_best: bool,
    checkpoint_best_metric: Option<String>,
    checkpoint_best_goal: String,
}

impl From<RetentionRow> for Retention {
    fn from(row: RetentionRow) -> Self {
        Self {
            days: row.checkpoint_days,
            keep_last: row.checkpoint_keep_last,
            keep_best: row.checkpoint_keep_best,
            best: row.checkpoint_best_metric.map(|metric| Best {
                metric,
                goal: Goal::parse(&row.checkpoint_best_goal),
            }),
        }
    }
}

pub async fn load(state: &AppState, project_id: i64) -> Result<Retention, ApiError> {
    let row: RetentionRow = sqlx::query_as(
        "SELECT checkpoint_days, checkpoint_keep_last, checkpoint_keep_best, checkpoint_best_metric,
                checkpoint_best_goal
         FROM projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_one(state.pg())
    .await?;
    Ok(row.into())
}

pub async fn save(state: &AppState, project_id: i64, r: &Retention) -> Result<(), ApiError> {
    r.check()?;
    sqlx::query(
        "UPDATE projects SET checkpoint_days = $2, checkpoint_keep_last = $3,
                checkpoint_best_metric = $4, checkpoint_best_goal = $5, checkpoint_keep_best = $6
         WHERE id = $1",
    )
    .bind(project_id)
    .bind(r.days)
    .bind(r.keep_last)
    .bind(r.best.as_ref().map(|b| b.metric.trim()))
    .bind(r.best.as_ref().map_or("max", |b| b.goal.as_str()))
    .bind(r.keep_best && r.best.is_some())
    .execute(state.pg())
    .await?;
    // Checkpoints past a shorter retention go now, not in a minute.
    state.clean_soon();
    Ok(())
}

/// A metric's latest finite value at or before a checkpoint's step.
#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
pub struct ValueAt {
    /// Where it was logged.
    pub step: u64,
    pub value: f64,
}

#[derive(Deserialize, clickhouse::Row)]
struct ValueRow {
    checkpoint: u64,
    key: String,
    step: u64,
    value: f64,
}

/// For each step in `steps` and metric in `keys`, the metric's latest finite value at or before
/// it (left out where there's none): `steps[i] -> key -> value`.
pub async fn values_at(
    state: &AppState,
    project_id: i64,
    run_id: Uuid,
    steps: &[u64],
    keys: &[String],
) -> Result<HashMap<u64, BTreeMap<String, ValueAt>>, ApiError> {
    let mut out: HashMap<u64, BTreeMap<String, ValueAt>> = HashMap::new();
    if steps.is_empty() || keys.is_empty() {
        return Ok(out);
    }
    let list = |n: usize| vec!["?"; n].join(", ");
    // ASOF: for each (step, key), the point of `key` with the greatest step <= it.
    let sql = format!(
        "SELECT c.checkpoint AS checkpoint, c.key AS key, m.step AS step, m.value AS value
         FROM (SELECT toUInt64(arrayJoin([{steps}])) AS checkpoint,
                      toString(arrayJoin([{keys}])) AS key) AS c
         ASOF JOIN (
             SELECT toString(key) AS key, step, value FROM metrics
             WHERE project_id = ? AND run_id = toUUID(?) AND key IN ({keys}) AND isFinite(value)
         ) AS m ON c.key = m.key AND c.checkpoint >= m.step",
        steps = list(steps.len()),
        keys = list(keys.len()),
    );
    let mut query = state.ch_read().query(&sql);
    for step in steps {
        query = query.bind(*step);
    }
    for key in keys {
        query = query.bind(key.as_str());
    }
    query = query.bind(project_id as u64).bind(run_id.to_string());
    for key in keys {
        query = query.bind(key.as_str());
    }
    for row in query
        .fetch_all::<ValueRow>()
        .await
        .map_err(ApiError::from)?
    {
        out.entry(row.checkpoint).or_default().insert(
            row.key,
            ValueAt {
                step: row.step,
                value: row.value,
            },
        );
    }
    Ok(out)
}

/// Why retention leaves a checkpoint alone.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Keep {
    Important,
    Last,
    Best,
}

/// A run's best checkpoint by `best`: where the metric's latest value at its step is best, the
/// first of them on a tie (a value logged once holds for every checkpoint until the next, and
/// describes the first of them, the one evaluated). None if the metric wasn't logged by then.
pub async fn best_of(
    state: &AppState,
    project_id: i64,
    run_id: Uuid,
    best: &Best,
    checkpoints: &[Stored],
) -> Result<Option<Uuid>, ApiError> {
    let steps: Vec<u64> = checkpoints.iter().map(|c| c.step as u64).collect();
    let values = values_at(
        state,
        project_id,
        run_id,
        &steps,
        std::slice::from_ref(&best.metric),
    )
    .await?;
    let mut by_step: Vec<&Stored> = checkpoints.iter().collect();
    by_step.sort_by_key(|c| c.step);
    let mut winner: Option<(Uuid, f64)> = None;
    for c in by_step {
        let Some(v) = values
            .get(&(c.step as u64))
            .and_then(|m| m.get(&best.metric))
        else {
            continue;
        };
        if winner.is_none_or(|(_, w)| best.goal.better(v.value, w)) {
            winner = Some((c.id, v.value));
        }
    }
    Ok(winner.map(|(id, _)| id))
}

/// A run's committed checkpoint, as retention sees it.
pub struct Stored {
    pub id: Uuid,
    pub step: i64,
    pub important: bool,
}

/// Which of a run's committed checkpoints `retention` protects, and why.
pub async fn protected(
    state: &AppState,
    project_id: i64,
    run_id: Uuid,
    retention: &Retention,
    checkpoints: &[Stored],
) -> Result<HashMap<Uuid, Vec<Keep>>, ApiError> {
    let mut keep: HashMap<Uuid, Vec<Keep>> = HashMap::new();
    for c in checkpoints.iter().filter(|c| c.important) {
        keep.entry(c.id).or_default().push(Keep::Important);
    }
    if retention.keep_last
        && let Some(last) = checkpoints.iter().max_by_key(|c| c.step)
    {
        keep.entry(last.id).or_default().push(Keep::Last);
    }
    if retention.keep_best
        && let Some(best) = &retention.best
        && let Some(id) = best_of(state, project_id, run_id, best, checkpoints).await?
    {
        keep.entry(id).or_default().push(Keep::Best);
    }
    Ok(keep)
}

/// About `n` of `steps`, spaced regularly by step: for each of `n` evenly spaced targets
/// between the first and the last, the nearest step (so both ends are kept; fewer than `n` where
/// steps bunch up).
pub fn spaced(steps: &[i64], n: usize) -> std::collections::HashSet<i64> {
    let mut sorted = steps.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.len() <= n {
        return sorted.into_iter().collect();
    }
    let mut out = std::collections::HashSet::new();
    let (first, last) = (sorted[0], *sorted.last().expect("not empty"));
    if n == 1 {
        out.insert(last);
        return out;
    }
    for i in 0..n {
        let target = first as f64 + (last - first) as f64 * i as f64 / (n - 1) as f64;
        let at = sorted.partition_point(|&s| (s as f64) < target);
        let nearest = match (at.checked_sub(1).map(|j| sorted[j]), sorted.get(at)) {
            (Some(below), Some(&above)) => {
                if target - below as f64 <= above as f64 - target {
                    below
                } else {
                    above
                }
            }
            (Some(below), None) => below,
            (None, Some(&above)) => above,
            (None, None) => continue,
        };
        out.insert(nearest);
    }
    out
}

/// A run's committed checkpoints.
pub async fn stored(state: &AppState, run_id: Uuid) -> Result<Vec<Stored>, ApiError> {
    let rows: Vec<(Uuid, i64, bool)> = sqlx::query_as(
        "SELECT id, step, important FROM checkpoints WHERE run_id = $1 AND state = 'committed'",
    )
    .bind(run_id)
    .fetch_all(state.pg())
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, step, important)| Stored {
            id,
            step,
            important,
        })
        .collect())
}

/// Deletes checkpoints past their project's retention, except the protected ones. Returns how
/// many. Their files go with the next garbage collection.
pub async fn apply(state: &AppState) -> Result<u64, ApiError> {
    // Runs with a checkpoint past retention that isn't marked important.
    let runs: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT DISTINCT r.id, r.project_id
         FROM checkpoints c JOIN runs r ON r.id = c.run_id JOIN projects p ON p.id = r.project_id
         WHERE c.state = 'committed' AND NOT c.important AND p.checkpoint_days IS NOT NULL
           AND c.committed_at < now() - make_interval(days => p.checkpoint_days)
         LIMIT $1",
    )
    .bind(BATCH)
    .fetch_all(state.pg())
    .await?;
    let mut deleted = 0;
    let mut retentions: HashMap<i64, Retention> = HashMap::new();
    for (run_id, project_id) in runs {
        let retention = match retentions.get(&project_id) {
            Some(r) => r.clone(),
            None => {
                let r = load(state, project_id).await?;
                retentions.insert(project_id, r.clone());
                r
            }
        };
        let Some(days) = retention.days else { continue };
        let checkpoints = stored(state, run_id).await?;
        let keep = protected(state, project_id, run_id, &retention, &checkpoints).await?;
        let kept: Vec<Uuid> = keep.into_keys().collect();
        let gone = sqlx::query(
            "DELETE FROM checkpoints
             WHERE run_id = $1 AND state = 'committed' AND NOT important AND NOT (id = ANY($2))
               AND committed_at < now() - make_interval(days => $3)",
        )
        .bind(run_id)
        .bind(&kept)
        .bind(days)
        .execute(state.pg())
        .await?
        .rows_affected();
        if gone > 0 {
            tracing::info!(run = %run_id, checkpoints = gone, days, "checkpoints past retention deleted");
        }
        deleted += gone;
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_is_checked() {
        let base = Retention {
            days: Some(30),
            keep_last: true,
            keep_best: false,
            best: None,
        };
        assert!(base.check().is_ok());
        for days in [0, -1, MAX_DAYS + 1] {
            let r = Retention {
                days: Some(days),
                ..base.clone()
            };
            assert!(r.check().is_err(), "{days}");
        }
        let r = Retention {
            best: Some(Best {
                metric: " ".into(),
                goal: Goal::Max,
            }),
            ..base
        };
        assert!(r.check().is_err(), "a metric is needed");
    }

    #[test]
    fn spacing_keeps_both_ends_and_regular_steps() {
        let steps: Vec<i64> = (1..=1000).map(|i| i * 10).collect();
        let kept = spaced(&steps, 100);
        assert_eq!(kept.len(), 100);
        assert!(kept.contains(&10) && kept.contains(&10_000));
        let mut sorted: Vec<i64> = kept.into_iter().collect();
        sorted.sort_unstable();
        let gaps: Vec<i64> = sorted.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().all(|&g| (90..=110).contains(&g)), "{gaps:?}");

        assert_eq!(spaced(&steps[..5], 100).len(), 5, "fewer than asked: all");
        assert_eq!(spaced(&steps, 1), [10_000].into(), "one: the last");
        assert!(spaced(&steps, 0).is_empty());
        // Bunched up: as many as there are distinct nearest ones.
        let bunched = [0, 1, 2, 3, 1000];
        assert_eq!(spaced(&bunched, 3), [0, 3, 1000].into());
    }

    #[test]
    fn goals_compare() {
        assert!(Goal::Max.better(2.0, 1.0) && !Goal::Max.better(1.0, 1.0));
        assert!(Goal::Min.better(1.0, 2.0) && !Goal::Min.better(2.0, 2.0));
    }
}
