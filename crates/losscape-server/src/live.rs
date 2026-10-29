//! Live updates for the UI: points of watched runs are pushed over SSE as soon as the
//! flusher has stored them (after dedup, so a retried batch is never shown twice).

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use losscape_core::proto::v0::ScalarColumns;
use serde::Serialize;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use uuid::Uuid;

use crate::auth::{self, AuthUser, Role};
use crate::read::{UiError, parse_run_id};
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
}

impl LiveEvent {
    fn run_id(&self) -> Uuid {
        match self {
            Self::Points { run_id, .. } | Self::State { run_id, .. } => *run_id,
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

    fn watch(self: &Arc<Self>, run_id: Uuid) -> (broadcast::Receiver<LiveEvent>, WatchGuard) {
        *self
            .watchers
            .lock()
            .expect("not poisoned")
            .entry(run_id)
            .or_default() += 1;
        (
            self.tx.subscribe(),
            WatchGuard {
                hub: self.clone(),
                run_id,
            },
        )
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
    keys: &'a [String],
    key_idx: &'a [u32],
    step: &'a [u64],
    value: Vec<Option<f64>>,
}

/// `GET /api/v0/runs/{id}/live`: events `points`, `state`, and `reset` (some events were
/// missed; reload the data).
pub async fn live(
    State(state): State<AppState>,
    user: AuthUser,
    Path(run): Path<String>,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, UiError> {
    let run_id = parse_run_id(&run)?;
    auth::require_run(&state, &user, run_id, Role::Viewer).await?;
    let (rx, guard) = state.live().watch(run_id);

    let events = BroadcastStream::new(rx).filter_map(move |item| {
        let _keep_watching = &guard;
        match item {
            Ok(event) if event.run_id() != run_id => None,
            Ok(LiveEvent::Points { scalars, .. }) => {
                let event = PointsEvent {
                    keys: &scalars.keys,
                    key_idx: &scalars.key_idx,
                    step: &scalars.step,
                    value: scalars
                        .value
                        .iter()
                        .map(|v| v.is_finite().then_some(*v))
                        .collect(),
                };
                Some(Ok(Event::default()
                    .event("points")
                    .json_data(event)
                    .expect("serializable")))
            }
            Ok(LiveEvent::State { state, .. }) => {
                Some(Ok(Event::default().event("state").data(state)))
            }
            Err(BroadcastStreamRecvError::Lagged(_)) => {
                Some(Ok(Event::default().event("reset").data("")))
            }
        }
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}
