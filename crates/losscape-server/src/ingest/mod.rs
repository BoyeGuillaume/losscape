//! `POST /api/v0/ingest`: validate a batch, queue it, answer once it is stored.

mod flusher;
pub mod window;

use std::sync::Arc;

use axum::extract::State;
use losscape_core::proto::v0::{Ack, ConsoleLine, ConsoleStream, LogBatch, ScalarColumns};
use uuid::Uuid;

pub use flusher::{IngestQueue, spawn_flusher};

use crate::auth::{self, AuthUser, Role};
use crate::error::ApiError;
use crate::live::LiveEvent;
use crate::proto_body::{
    MAX_CONSOLE_LINES_PER_BATCH, MAX_KEYS_PER_BATCH, MAX_ROWS_PER_BATCH, Proto,
};
use crate::state::AppState;

const MAX_KEY_LEN: usize = 256;
/// Longer console lines are cut (the client already cuts at 16 KB).
const MAX_CONSOLE_LINE_BYTES: usize = 64 * 1024;

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
    pub models_json: Option<String>,
    pub console: Vec<ConsoleLine>,
}

impl Batch {
    /// Points and console lines: what a flush writes.
    pub fn rows(&self) -> usize {
        self.scalars.value.len() + self.console.len()
    }
}

pub async fn ingest(
    State(state): State<AppState>,
    user: AuthUser,
    Proto(batch): Proto<LogBatch>,
) -> Result<Proto<Ack>, ApiError> {
    let run_id = parse_run_id(&batch.run_id)?;
    let mut scalars = batch.scalars.unwrap_or_default();
    strip_nul_from_keys(&mut scalars);
    validate_scalars(&scalars)?;
    let summary_json = json_object_or_none(batch.summary_json, "summary_json")?;
    let config_json = json_object_or_none(batch.config_json, "config_json")?;
    let models_json = json_object_or_none(batch.models_json, "models_json")?;
    let console = validate_console(batch.console)?;
    let project_id = auth::require_run(&state, &user, run_id, Role::Editor).await?;

    // Viewers get console lines now, not after the flush: they replace by line number, so a
    // retried batch shows nothing twice.
    if !console.is_empty() && state.live().is_watched(run_id) {
        state.live().publish(LiveEvent::Console {
            run_id,
            lines: Arc::new(console.clone()),
        });
    }

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
            models_json,
            console,
        })
        .await?;

    Ok(Proto(Ack {
        run_id: batch.run_id,
        seq: batch.seq,
        duplicate,
        abort: false,
    }))
}

pub fn parse_run_id(bytes: &[u8]) -> Result<Uuid, ApiError> {
    losscape_core::ids::RunId::from_bytes(bytes)
        .map(|id| Uuid::from_bytes(*id.as_bytes()))
        .ok_or_else(|| ApiError::invalid("run_id must be a 16-byte UUIDv7"))
}

fn validate_scalars(s: &ScalarColumns) -> Result<(), ApiError> {
    let rows = s.value.len();
    if rows > MAX_ROWS_PER_BATCH {
        return Err(ApiError::invalid(format!(
            "scalars: at most {MAX_ROWS_PER_BATCH} points per batch"
        )));
    }
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

fn validate_console(mut lines: Vec<ConsoleLine>) -> Result<Vec<ConsoleLine>, ApiError> {
    if lines.len() > MAX_CONSOLE_LINES_PER_BATCH {
        return Err(ApiError::invalid(format!(
            "console: at most {MAX_CONSOLE_LINES_PER_BATCH} lines per batch"
        )));
    }
    for line in &mut lines {
        if !matches!(line.stream(), ConsoleStream::Stdout | ConsoleStream::Stderr) {
            return Err(ApiError::invalid(
                "console: stream must be stdout or stderr",
            ));
        }
        if line.text.len() > MAX_CONSOLE_LINE_BYTES {
            let mut end = MAX_CONSOLE_LINE_BYTES;
            while !line.text.is_char_boundary(end) {
                end -= 1;
            }
            line.text.truncate(end);
        }
    }
    Ok(lines)
}

/// NUL is valid in protobuf strings and in JSON, but Postgres refuses it, in text and in jsonb.
/// It goes: nothing is lost that a person would miss, and a batch containing one would never
/// be stored, whichever batches share a transaction with it.
pub(crate) fn strip_nul(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\0') {
        text.replace('\0', "").into()
    } else {
        text.into()
    }
}

fn strip_nul_from_keys(s: &mut ScalarColumns) {
    for key in &mut s.keys {
        if key.contains('\0') {
            key.retain(|c| c != '\0');
        }
    }
}

fn strip_nul_from_json(value: &mut serde_json::Value) {
    use serde_json::Value;
    match value {
        Value::String(s) if s.contains('\0') => s.retain(|c| c != '\0'),
        Value::Array(items) => items.iter_mut().for_each(strip_nul_from_json),
        Value::Object(map) => {
            let old = std::mem::take(map);
            for (key, mut item) in old {
                strip_nul_from_json(&mut item);
                map.insert(strip_nul(&key).into_owned(), item);
            }
        }
        _ => {}
    }
}

/// Empty string means "nothing to merge"; anything else must be a JSON object.
pub fn json_object_or_none(raw: String, field: &str) -> Result<Option<String>, ApiError> {
    if raw.is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<serde_json::Value>(&raw) {
        // It can only be there written as an escape: a raw one is invalid JSON.
        Ok(mut value @ serde_json::Value::Object(_)) if raw.contains("\\u0000") => {
            strip_nul_from_json(&mut value);
            Ok(Some(value.to_string()))
        }
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
    fn console_lines_are_checked_and_cut() {
        let line = |stream: ConsoleStream, text: String| ConsoleLine {
            line: 0,
            time_ms: 0,
            stream: stream as i32,
            text,
        };
        let long = "é".repeat(MAX_CONSOLE_LINE_BYTES);
        let lines = validate_console(vec![line(ConsoleStream::Stderr, long)]).unwrap();
        assert!(lines[0].text.len() <= MAX_CONSOLE_LINE_BYTES);
        assert!(validate_console(vec![line(ConsoleStream::Unspecified, "x".into())]).is_err());
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

    #[test]
    fn nul_never_reaches_the_database() {
        let clean = json_object_or_none(
            r#"{"a\u0000b":"c\u0000d","n":{"e":["\u0000f"]},"ok":"\\u0000 literally"}"#.into(),
            "summary_json",
        )
        .unwrap()
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&clean).unwrap();
        assert_eq!(value["ab"], "cd");
        assert_eq!(value["n"]["e"][0], "f");
        assert_eq!(value["ok"], "\\u0000 literally");

        let mut columns = columns(1);
        columns.keys = vec!["lo\0ss".into()];
        strip_nul_from_keys(&mut columns);
        assert_eq!(columns.keys, ["loss"]);
    }
}
