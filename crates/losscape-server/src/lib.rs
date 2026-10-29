//! losscape API server.

pub mod app;
mod auth;
mod clickhouse_store;
pub mod config;
pub mod error;
pub mod ingest;
pub mod proto_body;
pub mod runs;
pub mod state;
#[cfg(feature = "testing")]
pub mod testing;
