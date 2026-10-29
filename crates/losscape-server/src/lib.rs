//! losscape API server.

pub mod app;
pub mod auth;
mod clickhouse_store;
pub mod config;
pub mod error;
pub mod ingest;
pub mod live;
pub mod projects;
pub mod proto_body;
pub mod read;
pub mod runs;
pub mod state;
#[cfg(feature = "testing")]
pub mod testing;
