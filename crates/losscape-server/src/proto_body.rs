//! Protobuf request/response bodies for the SDK endpoints.

use std::io::Read;

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use losscape_core::{PROTOCOL_HEADER, PROTOCOL_VERSION};
use prost::Message;

use crate::error::ApiError;

pub const CONTENT_TYPE: &str = "application/x-protobuf";

/// Upper bound on a body after zstd decompression.
const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;

pub struct Proto<T>(pub T);

impl<S, T> FromRequest<S> for Proto<T>
where
    S: Send + Sync,
    T: Message + Default,
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

        let message = if zstd {
            T::decode(decompress(&body)?.as_slice())
        } else {
            T::decode(body)
        };
        message
            .map(Proto)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_protobuf", e.to_string()))
    }
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
