use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::auth::AuthCaches;
use crate::auth::oauth::OAuthRuntime;
use crate::config::{AuthConfig, Config};
use crate::error::ApiError;
use crate::ingest::{IngestQueue, spawn_flusher};
use crate::live::LiveHub;

pub type StartError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

struct Inner {
    pg: PgPool,
    ch: clickhouse::Client,
    auth: AuthConfig,
    auth_caches: AuthCaches,
    oauth: OAuthRuntime,
    ingest: IngestQueue,
    live: Arc<LiveHub>,
    /// run id -> project id. Runs never change project, so entries never go stale.
    run_projects: RwLock<HashMap<Uuid, i64>>,
}

impl AppState {
    /// Connects to both stores, applies migrations and starts the ingest flusher.
    /// The flusher handle completes once every `AppState` clone is dropped and the
    /// queue is drained; await it on shutdown.
    pub async fn start(config: &Config) -> Result<(Self, JoinHandle<()>), StartError> {
        let pg = PgPoolOptions::new()
            .max_connections(16)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&config.database_url)
            .await?;
        sqlx::migrate!("./migrations/postgres").run(&pg).await?;
        let ch = crate::clickhouse_store::connect_and_migrate(&config.clickhouse).await?;

        let live = Arc::new(LiveHub::default());
        let (ingest, flusher) =
            spawn_flusher(pg.clone(), ch.clone(), live.clone(), config.ingest.clone());
        let state = Self(Arc::new(Inner {
            pg,
            ch,
            auth: config.auth.clone(),
            auth_caches: AuthCaches::default(),
            oauth: OAuthRuntime::default(),
            ingest,
            live,
            run_projects: RwLock::default(),
        }));
        Ok((state, flusher))
    }

    pub fn pg(&self) -> &PgPool {
        &self.0.pg
    }

    pub fn ch(&self) -> &clickhouse::Client {
        &self.0.ch
    }

    pub fn auth_config(&self) -> &AuthConfig {
        &self.0.auth
    }

    pub fn auth_caches(&self) -> &AuthCaches {
        &self.0.auth_caches
    }

    pub fn oauth(&self) -> &OAuthRuntime {
        &self.0.oauth
    }

    pub fn ingest(&self) -> &IngestQueue {
        &self.0.ingest
    }

    pub fn live(&self) -> &Arc<LiveHub> {
        &self.0.live
    }

    pub fn remember_run(&self, run_id: Uuid, project_id: i64) {
        self.0
            .run_projects
            .write()
            .expect("not poisoned")
            .insert(run_id, project_id);
    }

    pub async fn project_of(&self, run_id: Uuid) -> Result<i64, ApiError> {
        if let Some(&id) = self
            .0
            .run_projects
            .read()
            .expect("not poisoned")
            .get(&run_id)
        {
            return Ok(id);
        }
        let project_id: i64 = sqlx::query_scalar("SELECT project_id FROM runs WHERE id = $1")
            .bind(run_id)
            .fetch_optional(self.pg())
            .await?
            .ok_or_else(ApiError::unknown_run)?;
        self.remember_run(run_id, project_id);
        Ok(project_id)
    }
}
