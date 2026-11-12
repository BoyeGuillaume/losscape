//! losscape API server.

pub mod app;
pub mod auth;
pub mod board;
mod clickhouse_store;
pub mod config;
pub mod error;
pub mod history;
pub mod ingest;
pub mod live;
pub mod metric_defs;
pub mod projects;
pub mod proto_body;
pub mod purge;
pub mod read;
pub mod runs;
pub mod serve;
pub mod state;
pub mod tags;
#[cfg(feature = "testing")]
pub mod testing;
