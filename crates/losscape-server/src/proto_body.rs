//! Protobuf request/response bodies for the SDK endpoints.

use std::io::Read;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use losscape_core::proto::v0::{Heartbeat, LogBatch, RunFinish, RunInit};
use losscape_core::{PROTOCOL_HEADER, PROTOCOL_VERSION};
use prost::Message;
use tokio::sync::Semaphore;

use crate::error::ApiError;

pub const CONTENT_TYPE: &str = "application/x-protobuf";

/// Upper bound on a body after zstd decompression.
const MAX_DECODED_BYTES: u64 = 128 * 1024 * 1024;
/// Plain bodies up to this size are decoded right away.
const SMALL_BODY_BYTES: usize = 64 * 1024;
/// Most points in a batch; the SDK sends far fewer.
pub const MAX_ROWS_PER_BATCH: usize = 4_000_000;
pub const MAX_KEYS_PER_BATCH: usize = 10_000;
pub const MAX_CONSOLE_LINES_PER_BATCH: usize = 100_000;
const MAX_TAGS_ON_THE_WIRE: usize = 1_000;
/// Decoding is CPU work, and memory for as long as it lasts. A few at a time, off the async
/// workers; the others wait, and past this are told to retry.
static DECODING: Semaphore = Semaphore::const_new(4);
const DECODING_WAIT: Duration = Duration::from_secs(30);

pub struct Proto<T>(pub T);

impl<S, T> FromRequest<S> for Proto<T>
where
    S: Send + Sync,
    T: Message + Default + WireLimits + Send + 'static,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        check_headers(req.headers())?;
        let zstd = is_zstd(req.headers())?;

        let body = Bytes::from_request(req, state).await.map_err(|rejection| {
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                too_large()
            } else {
                ApiError::invalid(rejection.body_text())
            }
        })?;

        if !zstd && body.len() <= SMALL_BODY_BYTES {
            return decode(&body).map(Proto);
        }
        let _slot = tokio::time::timeout(DECODING_WAIT, DECODING.acquire())
            .await
            .map_err(|_| ApiError::busy())?
            .expect("never closed");
        tokio::task::spawn_blocking(move || {
            if zstd {
                decode(&decompress(&body)?)
            } else {
                decode(&body)
            }
        })
        .await
        .map_err(|_| ApiError::internal())?
        .map(Proto)
    }
}

fn decode<T: Message + Default + WireLimits>(bytes: &[u8]) -> Result<T, ApiError> {
    T::check(bytes)?;
    T::decode(bytes)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_protobuf", e.to_string()))
}

/// Checks on the encoded message, before any of it is allocated. Decoded, a message can take
/// far more memory than encoded: an empty console line is 2 bytes on the wire and 48 in memory.
pub trait WireLimits {
    fn check(_bytes: &[u8]) -> Result<(), ApiError> {
        Ok(())
    }
}

impl WireLimits for Heartbeat {}
impl WireLimits for RunFinish {}

impl WireLimits for RunInit {
    fn check(bytes: &[u8]) -> Result<(), ApiError> {
        let mut tags = 0;
        walk(bytes, |field, _| tags += usize::from(field == 5))?;
        if tags > MAX_TAGS_ON_THE_WIRE {
            return Err(too_large());
        }
        Ok(())
    }
}

impl WireLimits for LogBatch {
    fn check(bytes: &[u8]) -> Result<(), ApiError> {
        let (mut console, mut keys) = (0, 0);
        let mut rows = [0; 4]; // key_idx, step, time_ms, value
        let mut scalars = Ok(());
        walk(bytes, |field, value| match (field, value) {
            (7, _) => console += 1,
            (3, Value::Bytes(columns)) => {
                let walked = walk(columns, |field, value| match (field, value) {
                    (1, _) => keys += 1,
                    (2..=4, Value::Bytes(packed)) => {
                        rows[field as usize - 2] += count_varints(packed);
                    }
                    (5, Value::Bytes(packed)) => rows[3] += packed.len() / 8,
                    (2..=5, _) => rows[field as usize - 2] += 1,
                    _ => {}
                });
                scalars = scalars.clone().and(walked);
            }
            _ => {}
        })?;
        scalars?;
        if console > MAX_CONSOLE_LINES_PER_BATCH
            || keys > MAX_KEYS_PER_BATCH
            || rows.iter().any(|&n| n > MAX_ROWS_PER_BATCH)
        {
            return Err(too_large());
        }
        Ok(())
    }
}

enum Value<'a> {
    Bytes(&'a [u8]),
    Scalar,
}

fn count_varints(packed: &[u8]) -> usize {
    packed.iter().filter(|&&b| b & 0x80 == 0).count()
}

/// Calls `visit` with the number and value of each field of an encoded message.
fn walk(mut bytes: &[u8], mut visit: impl FnMut(u32, Value<'_>)) -> Result<(), ApiError> {
    let malformed = || ApiError::new(StatusCode::BAD_REQUEST, "invalid_protobuf", "malformed");
    while !bytes.is_empty() {
        let tag = read_varint(&mut bytes).ok_or_else(malformed)?;
        let field = u32::try_from(tag >> 3).map_err(|_| malformed())?;
        let skip = match tag & 7 {
            0 => {
                read_varint(&mut bytes).ok_or_else(malformed)?;
                0
            }
            1 => 8,
            5 => 4,
            2 => {
                let len = read_varint(&mut bytes).ok_or_else(malformed)?;
                let len = usize::try_from(len).map_err(|_| malformed())?;
                let value = bytes.get(..len).ok_or_else(malformed)?;
                visit(field, Value::Bytes(value));
                bytes = &bytes[len..];
                continue;
            }
            // Groups: long deprecated, and nothing here uses them.
            _ => return Err(malformed()),
        };
        bytes = bytes.get(skip..).ok_or_else(malformed)?;
        visit(field, Value::Scalar);
    }
    Ok(())
}

fn read_varint(bytes: &mut &[u8]) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let (&byte, rest) = bytes.split_first()?;
        *bytes = rest;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

impl<T: Message> IntoResponse for Proto<T> {
    fn into_response(self) -> Response {
        (
            [(header::CONTENT_TYPE, CONTENT_TYPE)],
            self.0.encode_to_vec(),
        )
            .into_response()
    }
}

fn check_headers(headers: &HeaderMap) -> Result<(), ApiError> {
    let protocol = headers.get(PROTOCOL_HEADER).and_then(|v| v.to_str().ok());
    if protocol != Some(&PROTOCOL_VERSION.to_string()) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unsupported_protocol",
            format!("this server speaks protocol {PROTOCOL_VERSION} (send {PROTOCOL_HEADER})"),
        ));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    if !content_type.is_some_and(|ct| ct.starts_with(CONTENT_TYPE)) {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            format!("expected Content-Type: {CONTENT_TYPE}"),
        ));
    }
    Ok(())
}

fn is_zstd(headers: &HeaderMap) -> Result<bool, ApiError> {
    match headers.get(header::CONTENT_ENCODING).map(|v| v.to_str()) {
        None => Ok(false),
        Some(Ok("zstd")) => Ok(true),
        Some(Ok("identity")) => Ok(false),
        Some(_) => Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_encoding",
            "Content-Encoding must be zstd or absent",
        )),
    }
}

fn decompress(body: &[u8]) -> Result<Vec<u8>, ApiError> {
    let decoder = zstd::stream::read::Decoder::new(body)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_zstd", e.to_string()))?;
    let mut out = Vec::with_capacity(body.len() * 4);
    decoder
        .take(MAX_DECODED_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_zstd", e.to_string()))?;
    if out.len() as u64 > MAX_DECODED_BYTES {
        return Err(too_large());
    }
    Ok(out)
}

fn too_large() -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
        "request body too large",
    )
}

#[cfg(test)]
mod tests {
    use losscape_core::proto::v0::{ConsoleLine, ScalarColumns};

    use super::*;

    fn code(result: Result<(), ApiError>) -> &'static str {
        result.err().map_or("ok", |e| e.code)
    }

    #[test]
    fn a_batch_is_checked_before_it_is_allocated() {
        let batch = |console: usize, keys: usize, rows: usize| {
            LogBatch {
                console: vec![ConsoleLine::default(); console],
                scalars: Some(ScalarColumns {
                    keys: vec!["k".into(); keys],
                    key_idx: vec![0; rows],
                    step: vec![1; rows],
                    time_ms: vec![1; rows],
                    value: vec![1.0; rows],
                }),
                ..Default::default()
            }
            .encode_to_vec()
        };
        assert_eq!(code(LogBatch::check(&batch(10, 10, 10))), "ok");
        assert_eq!(
            code(LogBatch::check(&batch(MAX_CONSOLE_LINES_PER_BATCH, 1, 1))),
            "ok"
        );
        for too_many in [
            batch(MAX_CONSOLE_LINES_PER_BATCH + 1, 1, 1),
            batch(1, MAX_KEYS_PER_BATCH + 1, 1),
            batch(1, 1, MAX_ROWS_PER_BATCH + 1),
        ] {
            assert_eq!(code(LogBatch::check(&too_many)), "payload_too_large");
        }
    }

    #[test]
    fn empty_console_lines_are_counted_as_they_come() {
        // The cheapest way to make a decoder allocate: 2 bytes each.
        let bomb = [0x3a, 0x00].repeat(MAX_CONSOLE_LINES_PER_BATCH + 1);
        assert_eq!(code(LogBatch::check(&bomb)), "payload_too_large");
    }

    #[test]
    fn columns_split_over_several_fields_add_up() {
        // Unpacked, and spread over several `scalars`: what a decoder would merge.
        let one = ScalarColumns {
            step: vec![1; MAX_ROWS_PER_BATCH / 2 + 1],
            ..Default::default()
        }
        .encode_to_vec();
        let mut message = Vec::new();
        for _ in 0..2 {
            message.push(0x1a);
            prost::encoding::encode_varint(one.len() as u64, &mut message);
            message.extend_from_slice(&one);
        }
        assert_eq!(code(LogBatch::check(&message)), "payload_too_large");
    }

    #[test]
    fn malformed_messages_are_refused_rather_than_panicking() {
        for bytes in [
            &[0x08][..],             // a varint that isn't there
            &[0x0a, 0x05, 0x01][..], // longer than what's left
            &[0x0b][..],             // a group
            &[0xff; 11][..],         // a varint that never ends
            &[0x09, 0x01][..],       // 8 bytes expected
        ] {
            assert_eq!(code(LogBatch::check(bytes)), "invalid_protobuf");
        }
    }
}
