//! losscape API server.

pub mod app;
pub mod auth;
pub mod board;
pub mod checkpoints;
mod clickhouse_store;
pub mod config;
pub mod error;
pub mod hashing;
pub mod history;
pub mod ingest;
pub mod links;
pub mod live;
pub mod metric_defs;
pub mod project_requests;
pub mod projects;
pub mod proto_body;
pub mod purge;
pub mod read;
pub mod retention;
pub mod runs;
pub mod serve;
pub mod state;
pub mod storage;
pub mod tags;
mod tar;
#[cfg(feature = "testing")]
pub mod testing;
pub mod tls;
