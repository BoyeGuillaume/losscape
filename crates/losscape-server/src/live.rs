//! Live updates for the UI: points of watched runs are pushed over SSE as soon as the
//! flusher has stored them (after dedup, so a retried batch is never shown twice), console
//! lines as soon as they arrive. One stream covers many runs, since browsers allow only a
//! few connections per server.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use losscape_core::proto::v0::{ConsoleLine, ScalarColumns};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use uuid::Uuid;

use crate::auth::{self, AuthUser, Role, members};
use crate::read::{UiError, parse_run_ids, unknown_project};
use crate::state::AppState;

const CHANNEL_CAPACITY: usize = 1024;

#[derive(Clone, Debug)]
pub enum LiveEvent {
    Points {
        run_id: Uuid,
        scalars: Arc<ScalarColumns>,
    },
    State {
        run_id: Uuid,
        state: &'static str,
    },
    Console {
        run_id: Uuid,
        lines: Arc<Vec<ConsoleLine>>,
    },
}

impl LiveEvent {
    fn run_id(&self) -> Uuid {
        match self {
            Self::Points { run_id, .. }
            | Self::State { run_id, .. }
            | Self::Console { run_id, .. } => *run_id,
        }
    }
}

pub struct LiveHub {
    tx: broadcast::Sender<LiveEvent>,
    /// Viewers per run; only watched runs are published.
    watchers: Mutex<HashMap<Uuid, usize>>,
}

impl Default for LiveHub {
    fn default() -> Self {
        Self {
            tx: broadcast::channel(CHANNEL_CAPACITY).0,
            watchers: Mutex::default(),
        }
    }
}

impl LiveHub {
    pub fn is_watched(&self, run_id: Uuid) -> bool {
        self.watchers
            .lock()
            .expect("not poisoned")
            .contains_key(&run_id)
    }

    pub fn publish(&self, event: LiveEvent) {
        if self.is_watched(event.run_id()) {
            // No receivers left is fine.
            let _ = self.tx.send(event);
        }
    }

    fn watch(self: &Arc<Self>, run_id: Uuid) -> WatchGuard {
        *self
            .watchers
            .lock()
            .expect("not poisoned")
            .entry(run_id)
            .or_default() += 1;
        WatchGuard {
            hub: self.clone(),
            run_id,
        }
    }
}

struct WatchGuard {
    hub: Arc<LiveHub>,
    run_id: Uuid,
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        let mut watchers = self.hub.watchers.lock().expect("not poisoned");
        if let Some(count) = watchers.get_mut(&self.run_id) {
            *count -= 1;
            if *count == 0 {
                watchers.remove(&self.run_id);
            }
        }
    }
}

/// One SSE `points` event: same columnar shape as the ingest protocol. Non-finite
/// values are sent as null.
#[derive(Serialize)]
struct PointsEvent<'a> {
    run: Uuid,
    keys: &'a [String],
    key_idx: &'a [u32],
    step: &'a [u64],
    time_ms: &'a [i64],
    value: Vec<Option<f64>>,
}

/// One SSE `console` event: lines that are new or replace earlier versions.
#[derive(Serialize)]
struct ConsoleEvent<'a> {
    run: Uuid,
    lines: Vec<crate::read::LogLine<'a>>,
}

#[derive(Serialize)]
struct StateEvent {
    run: Uuid,
    state: &'static str,
}

#[derive(Deserialize)]
pub struct LiveParams {
    /// Comma-separated run ids.
    runs: String,
}

/// `GET /api/v0/projects/{project}/live?runs=<id>,<id>`: events `points`, `console` and
/// `state` (JSON, each with its `run`), and `reset` (some events were missed; reload). Runs of
/// other projects are ignored.
pub async fn live(
    State(state): State<AppState>,
    user: AuthUser,
    Path(project): Path<String>,
    Query(params): Query<LiveParams>,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, UiError> {
    let project_id = members::project_id(&state, &project).await?;
    auth::require_role(&state, &user, project_id, Role::Viewer, unknown_project).await?;
    let requested: Vec<String> = params
        .runs
        .split(',')
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
        .collect();
    let requested = parse_run_ids(&requested)?;
    let runs: HashSet<Uuid> =
        sqlx::query_scalar("SELECT id FROM runs WHERE project_id = $1 AND id = ANY($2)")
            .bind(project_id)
            .bind(&requested)
            .fetch_all(state.pg())
            .await?
            .into_iter()
            .collect();
    let rx = state.live().tx.subscribe();
    let guards: Vec<WatchGuard> = runs.iter().map(|&run| state.live().watch(run)).collect();

    let events = BroadcastStream::new(rx).filter_map(move |item| {
        let _keep_watching = &guards;
        let event = match item {
            Ok(event) if !runs.contains(&event.run_id()) => return None,
            Ok(LiveEvent::Points { run_id, scalars }) => {
                Event::default().event("points").json_data(PointsEvent {
                    run: run_id,
                    keys: &scalars.keys,
                    key_idx: &scalars.key_idx,
                    step: &scalars.step,
                    time_ms: &scalars.time_ms,
                    value: scalars
                        .value
                        .iter()
                        .map(|v| v.is_finite().then_some(*v))
                        .collect(),
                })
            }
            Ok(LiveEvent::Console { run_id, lines }) => {
                Event::default().event("console").json_data(ConsoleEvent {
                    run: run_id,
                    lines: lines.iter().map(crate::read::LogLine::from).collect(),
                })
            }
            Ok(LiveEvent::State { run_id, state }) => Event::default()
                .event("state")
                .json_data(StateEvent { run: run_id, state }),
            Err(BroadcastStreamRecvError::Lagged(_)) => {
                Ok(Event::default().event("reset").data(""))
            }
        };
        Some(Ok(event.expect("serializable")))
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}
