use std::collections::HashMap;
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;

use axum::http::StatusCode;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use subtle::ConstantTimeEq;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::auth::oauth::OAuthRuntime;
use crate::auth::{AuthCaches, secrets};
use crate::config::{AuthConfig, Branding, Config, SetupToken};
use crate::error::ApiError;
use crate::ingest::{IngestQueue, spawn_flusher};
use crate::live::LiveHub;

/// Limits of one query made on behalf of a user: bytes, and seconds.
const CLICKHOUSE_READ_MEMORY: &str = "2000000000";
const CLICKHOUSE_READ_SECONDS: &str = "60";

async fn setup_token(pg: &PgPool, setting: &SetupToken) -> Result<Option<String>, StartError> {
    if matches!(setting, SetupToken::NotRequired) {
        return Ok(None);
    }
    let accounts: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users)")
        .fetch_one(pg)
        .await?;
    Ok(match setting {
        _ if accounts => None,
        SetupToken::Fixed(token) => Some(token.clone()),
        _ => {
            let token = secrets::random_token();
            // Not through the log filter: this has to be seen, and only ever goes out while
            // there's nothing yet to protect but this.
            eprintln!(
                "\n  No accounts yet. To create the first one, the administrator, use this setup token:\n\n      {token}\n"
            );
            Some(token)
        }
    })
}

pub type StartError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

/// For what watches over something else in the background, and shouldn't keep the server up.
#[derive(Clone)]
pub struct WeakAppState(Weak<Inner>);

impl WeakAppState {
    pub fn upgrade(&self) -> Option<AppState> {
        self.0.upgrade().map(AppState)
    }
}

struct Inner {
    pg: PgPool,
    ch: clickhouse::Client,
    ch_read: clickhouse::Client,
    auth: AuthConfig,
    auth_caches: AuthCaches,
    oauth: OAuthRuntime,
    ingest: IngestQueue,
    live: Arc<LiveHub>,
    branding: Branding,
    live_recheck: Duration,
    /// Asked for when creating the first account; None once there are accounts.
    setup_token: Option<String>,
    /// run id -> project id. Runs never change project; entries go when runs are deleted.
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

        let setup_token = setup_token(&pg, &config.auth.setup_token).await?;
        let ch_read = ch
            .clone()
            .with_option("max_memory_usage", CLICKHOUSE_READ_MEMORY)
            .with_option("max_execution_time", CLICKHOUSE_READ_SECONDS);
        let live = Arc::new(LiveHub::default());
        let (ingest, flusher) =
            spawn_flusher(pg.clone(), ch.clone(), live.clone(), config.ingest.clone());
        let state = Self(Arc::new(Inner {
            pg,
            ch,
            ch_read,
            auth: config.auth.clone(),
            auth_caches: AuthCaches::default(),
            oauth: OAuthRuntime::default(),
            ingest,
            live,
            branding: config.branding.clone(),
            live_recheck: config.live_recheck,
            setup_token,
            run_projects: RwLock::default(),
        }));
        Ok((state, flusher))
    }

    pub fn downgrade(&self) -> WeakAppState {
        WeakAppState(Arc::downgrade(&self.0))
    }

    pub fn live_recheck(&self) -> Duration {
        self.0.live_recheck
    }

    pub fn setup_token(&self) -> Option<&str> {
        self.0.setup_token.as_deref()
    }

    /// Whoever reaches a new server first can't become its administrator just by being first:
    /// they have to be able to see its log, or to have been given the token.
    pub fn check_setup_token(&self, given: Option<&str>) -> Result<(), ApiError> {
        let Some(expected) = self.setup_token() else {
            return Ok(());
        };
        let given = given.unwrap_or_default().trim();
        if bool::from(given.as_bytes().ct_eq(expected.as_bytes())) {
            return Ok(());
        }
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "setup_token_required",
            "the first account needs the setup token from the server's log",
        ))
    }

    pub fn pg(&self) -> &PgPool {
        &self.0.pg
    }

    pub fn ch(&self) -> &clickhouse::Client {
        &self.0.ch
    }

    /// For what users ask to read: within limits, so one request can't take over ClickHouse.
    pub fn ch_read(&self) -> &clickhouse::Client {
        &self.0.ch_read
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

    pub fn branding(&self) -> &Branding {
        &self.0.branding
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

    /// Drops cached runs (deleted runs, or every run of a deleted project).
    pub fn forget_runs(&self, gone: impl Fn(&Uuid, &i64) -> bool) {
        self.0
            .run_projects
            .write()
            .expect("not poisoned")
            .retain(|run, project| !gone(run, project));
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
