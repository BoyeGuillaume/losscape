//! `POST /api/v0/ingest`: validate a batch, queue it, answer once it is stored.

mod flusher;
pub mod window;

use axum::extract::State;
use losscape_core::proto::v0::{Ack, LogBatch, ScalarColumns};
use uuid::Uuid;

pub use flusher::{IngestQueue, spawn_flusher};

use crate::error::ApiError;
use crate::proto_body::Proto;
use crate::state::AppState;

const MAX_KEY_LEN: usize = 256;
const MAX_KEYS_PER_BATCH: usize = 10_000;

/// A validated batch waiting to be written.
#[derive(Debug)]
pub struct Batch {
    pub run_id: Uuid,
    pub project_id: i64,
    pub seq: u64,
    pub confirmed_below: u64,
    pub scalars: ScalarColumns,
    pub summary_json: Option<String>,
    pub config_json: Option<String>,
}

impl Batch {
    pub fn rows(&self) -> usize {
        self.scalars.value.len()
    }
}

pub async fn ingest(
    State(state): State<AppState>,
    Proto(batch): Proto<LogBatch>,
) -> Result<Proto<Ack>, ApiError> {
    let run_id = parse_run_id(&batch.run_id)?;
    let scalars = batch.scalars.unwrap_or_default();
    validate_scalars(&scalars)?;
    let summary_json = json_object_or_none(batch.summary_json, "summary_json")?;
    let config_json = json_object_or_none(batch.config_json, "config_json")?;
    let project_id = state.project_of(run_id).await?;

    let duplicate = state
        .ingest()
        .submit(Batch {
            run_id,
            project_id,
            seq: batch.seq,
            confirmed_below: batch.confirmed_below,
            scalars,
            summary_json,
            config_json,
        })
        .await?;

    Ok(Proto(Ack {
        run_id: batch.run_id,
        seq: batch.seq,
        duplicate,
    }))
}

pub fn parse_run_id(bytes: &[u8]) -> Result<Uuid, ApiError> {
    losscape_core::ids::RunId::from_bytes(bytes)
        .map(|id| Uuid::from_bytes(*id.as_bytes()))
        .ok_or_else(|| ApiError::invalid("run_id must be a 16-byte UUIDv7"))
}

fn validate_scalars(s: &ScalarColumns) -> Result<(), ApiError> {
    let rows = s.value.len();
    if s.key_idx.len() != rows || s.step.len() != rows || s.time_ms.len() != rows {
        return Err(ApiError::invalid(
            "scalars: key_idx, step, time_ms and value must have the same length",
        ));
    }
    if s.keys.len() > MAX_KEYS_PER_BATCH {
        return Err(ApiError::invalid(format!(
            "scalars: at most {MAX_KEYS_PER_BATCH} keys per batch"
        )));
    }
    if let Some(key) = s
        .keys
        .iter()
        .find(|k| k.is_empty() || k.len() > MAX_KEY_LEN)
    {
        return Err(ApiError::invalid(format!(
            "scalars: keys must be 1-{MAX_KEY_LEN} bytes, got {:?}",
            key.chars().take(40).collect::<String>()
        )));
    }
    let keys = s.keys.len() as u32;
    if s.key_idx.iter().any(|&i| i >= keys) {
        return Err(ApiError::invalid("scalars: key_idx out of range"));
    }
    Ok(())
}

/// Empty string means "nothing to merge"; anything else must be a JSON object.
pub fn json_object_or_none(raw: String, field: &str) -> Result<Option<String>, ApiError> {
    if raw.is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(serde_json::Value::Object(_)) => Ok(Some(raw)),
        Ok(_) => Err(ApiError::invalid(format!("{field} must be a JSON object"))),
        Err(e) => Err(ApiError::invalid(format!("{field}: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns(n: usize) -> ScalarColumns {
        ScalarColumns {
            keys: vec!["loss".into()],
            key_idx: vec![0; n],
            step: (0..n as u64).collect(),
            time_ms: vec![0; n],
            value: vec![1.0; n],
        }
    }

    #[test]
    fn accepts_valid_columns() {
        assert!(validate_scalars(&columns(3)).is_ok());
        assert!(validate_scalars(&ScalarColumns::default()).is_ok());
    }

    #[test]
    fn rejects_ragged_columns() {
        let mut c = columns(3);
        c.step.pop();
        assert!(validate_scalars(&c).is_err());
    }

    #[test]
    fn rejects_bad_key_index_and_names() {
        let mut c = columns(2);
        c.key_idx[1] = 1;
        assert!(validate_scalars(&c).is_err());

        let mut c = columns(1);
        c.keys[0] = String::new();
        assert!(validate_scalars(&c).is_err());
    }

    #[test]
    fn json_objects_only() {
        assert_eq!(json_object_or_none(String::new(), "f").unwrap(), None);
        assert!(
            json_object_or_none(r#"{"a":1}"#.into(), "f")
                .unwrap()
                .is_some()
        );
        assert!(json_object_or_none("[1]".into(), "f").is_err());
        assert!(json_object_or_none("{".into(), "f").is_err());
    }
}
