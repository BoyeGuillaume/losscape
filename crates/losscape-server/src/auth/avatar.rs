//! Profile pictures.
//!
//! Uploads are never stored as sent: the format is detected from the bytes (PNG, JPEG or
//! WebP only; SVG and anything else is refused), dimensions and memory are capped before
//! decoding, and the image is center-cropped to a square and re-encoded as a fresh PNG.
//! Re-encoding drops metadata (e.g. photo GPS location) and anything hidden in the file.

use std::io::Cursor;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use image::imageops::FilterType;
use image::{ImageFormat, ImageReader, Limits};

use super::AuthUser;
use crate::error::ApiError;
use crate::read::UiError;
use crate::state::AppState;

/// Stored size, in pixels (square).
pub const AVATAR_SIZE: u32 = 256;
/// Largest upload accepted, before decoding.
pub const MAX_UPLOAD_BYTES: usize = 2 * 1024 * 1024;
/// Largest source image accepted, per side.
const MAX_SOURCE_SIDE: u32 = 4096;
const MAX_DECODE_ALLOC: u64 = 64 * 1024 * 1024;

fn invalid_image(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid_image", message)
}

/// Validates and normalizes an upload into a square PNG of `AVATAR_SIZE` pixels.
pub fn normalize(bytes: &[u8]) -> Result<Vec<u8>, ApiError> {
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "images up to 2 MB",
        ));
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid_image("unreadable image"))?;
    match reader.format() {
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP) => {}
        _ => return Err(invalid_image("use a PNG, JPEG or WebP image")),
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| {
        invalid_image(format!(
            "could not decode the image (at most {MAX_SOURCE_SIDE}x{MAX_SOURCE_SIDE}): {e}"
        ))
    })?;

    let square = image.resize_to_fill(AVATAR_SIZE, AVATAR_SIZE, FilterType::Lanczos3);
    let mut png = Vec::new();
    square
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|_| ApiError::internal())?;
    Ok(png)
}

/// `PUT /api/v0/me/avatar`: raw image bytes as the body.
pub async fn upload(
    State(state): State<AppState>,
    user: AuthUser,
    body: Bytes,
) -> Result<StatusCode, UiError> {
    let png = tokio::task::spawn_blocking(move || normalize(&body))
        .await
        .map_err(|_| ApiError::internal())??;
    sqlx::query(
        "INSERT INTO user_avatars (user_id, image) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET image = EXCLUDED.image, updated_at = now()",
    )
    .bind(user.id)
    .bind(png)
    .execute(state.pg())
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v0/me/avatar`.
pub async fn remove(State(state): State<AppState>, user: AuthUser) -> Result<StatusCode, UiError> {
    sqlx::query("DELETE FROM user_avatars WHERE user_id = $1")
        .bind(user.id)
        .execute(state.pg())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v0/users/{username}/avatar` (signed-in users). URLs carry a version query
/// (`?v=<updated ms>`), so responses can be cached for long.
pub async fn serve(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(username): Path<String>,
) -> Result<Response, UiError> {
    let image: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT a.image FROM user_avatars a JOIN users u ON u.id = a.user_id WHERE lower(u.username) = lower($1)",
    )
    .bind(&username)
    .fetch_optional(state.pg())
    .await?;
    let image = image
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "no_avatar", "no profile picture"))?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "private, max-age=86400"),
        ],
        image,
    )
        .into_response())
}

/// SQL for a user's avatar URL (NULL without a picture), given `u` = users and `a` =
/// user_avatars LEFT JOINed. Usernames are restricted to URL-safe characters.
pub const AVATAR_URL_SQL: &str = "CASE WHEN a.user_id IS NULL THEN NULL
     ELSE '/api/v0/users/' || u.username || '/avatar?v=' || (extract(epoch from a.updated_at) * 1000)::bigint END";

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(image: image::DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Vec::new();
        image.write_to(&mut Cursor::new(&mut out), format).unwrap();
        out
    }

    #[test]
    fn crops_and_reencodes_to_a_square_png() {
        let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(600, 400, |x, _| {
            image::Rgb([x as u8, 0, 0])
        }));
        for format in [ImageFormat::Png, ImageFormat::Jpeg] {
            let png = normalize(&encode(source.clone(), format)).unwrap();
            let decoded = image::load_from_memory(&png).unwrap();
            assert_eq!(
                (decoded.width(), decoded.height()),
                (AVATAR_SIZE, AVATAR_SIZE)
            );
            assert_eq!(image::guess_format(&png).unwrap(), ImageFormat::Png);
        }
    }

    #[test]
    fn refuses_other_formats_and_huge_images() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
        assert_eq!(normalize(svg).unwrap_err().code, "invalid_image");
        assert_eq!(
            normalize(b"<html><script>alert(1)</script>")
                .unwrap_err()
                .code,
            "invalid_image"
        );
        let png = encode(image::DynamicImage::new_rgb8(10, 10), ImageFormat::Png);
        assert!(normalize(&png[..20]).is_err(), "truncated");
        let wide = encode(
            image::DynamicImage::new_luma8(MAX_SOURCE_SIDE + 1, 2),
            ImageFormat::Png,
        );
        assert_eq!(normalize(&wide).unwrap_err().code, "invalid_image");
        assert_eq!(
            normalize(&vec![0u8; MAX_UPLOAD_BYTES + 1])
                .unwrap_err()
                .code,
            "payload_too_large"
        );
    }
}
