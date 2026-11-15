//! Checkpoints: folders a run uploads at a step, kept in the configured storage
//! (docs/ARTIFACTS.md). One is created from its manifest, its files are uploaded in parts (each
//! checked against its hash), and it's committed once they're all there. All of it goes through
//! the server, which checks who asks: storage is never reached directly.

use std::collections::HashSet;
use std::time::Duration;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::{self, AuthUser, Role};
use crate::error::ApiError;
use crate::read::{UiError, parse_run_id};
use crate::state::AppState;
use crate::storage::Storage;

/// Bytes of a file per part, before compression.
pub const PART_SIZE: u64 = 16 * 1024 * 1024;
/// Most a part can take stored: zstd's worst case on data it can't compress, and some room.
pub const MAX_STORED_PART: usize = PART_SIZE as usize + PART_SIZE as usize / 64 + 64 * 1024;
pub const MAX_FILES: usize = 10_000;
const MAX_PATH_BYTES: usize = 1024;
const MAX_NAME_BYTES: usize = 255;
/// Pending checkpoints never committed go after this long, with what was uploaded.
const PENDING_HOURS: i32 = 24;
const CLEAN_EVERY: Duration = Duration::from_secs(60);
/// Header with the SHA-256 of a part, as sent (hex).
const PART_HASH: &str = "x-losscape-sha256";

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    None,
    Zstd,
}

impl Compression {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Zstd => "zstd",
        }
    }

    fn parse(s: &str) -> Self {
        if s == "zstd" { Self::Zstd } else { Self::None }
    }
}

#[derive(Deserialize)]
pub struct NewCheckpoint {
    step: u64,
    files: Vec<NewFile>,
}

#[derive(Deserialize)]
pub struct NewFile {
    path: String,
    size: u64,
    /// Of the file as it is, before compression (hex).
    sha256: String,
    compression: Compression,
    /// `size` in parts of `PART_SIZE` (the last one shorter).
    parts: u32,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Checkpoint {
    id: Uuid,
    run: Uuid,
    step: i64,
    /// `pending` while files are uploaded, then `committed`.
    state: String,
    size: i64,
    /// As stored, compressed or not: what's uploaded so far while pending.
    stored_size: i64,
    file_count: i64,
    created_at_ms: i64,
    committed_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[sqlx(skip)]
    files: Option<Vec<FileInfo>>,
}

#[derive(Serialize)]
pub struct FileInfo {
    index: i32,
    path: String,
    size: i64,
    stored_size: i64,
    sha256: String,
    compression: Compression,
    parts: i32,
    /// Parts received, while pending: the rest is still to send.
    #[serde(skip_serializing_if = "Option::is_none")]
    uploaded: Option<Vec<i32>>,
}

/// What a file is checked against.
#[derive(PartialEq, Debug)]
struct Declared {
    path: String,
    size: i64,
    sha256: Vec<u8>,
    compression: Compression,
    parts: i32,
}

fn storage(state: &AppState) -> Result<&Storage, ApiError> {
    state.storage().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "no_storage",
            "this server keeps no checkpoints: set LOSSCAPE_STORAGE",
        )
    })
}

fn unknown_checkpoint() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "unknown_checkpoint",
        "no such checkpoint",
    )
}

fn conflict(code: &'static str, message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, code, message)
}

/// Relative, `/`-separated, without empty, `.` or `..` parts, backslashes or control
/// characters: a path that means the same everywhere it's written back to disk.
fn valid_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return Err(format!("1-{MAX_PATH_BYTES} bytes"));
    }
    if path.chars().any(|c| c.is_control() || c == '\\') {
        return Err("no control characters or backslashes".into());
    }
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err("relative, without empty, . or .. parts".into());
        }
        if part.len() > MAX_NAME_BYTES {
            return Err(format!("names of at most {MAX_NAME_BYTES} bytes"));
        }
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

fn parts_of(size: u64) -> u64 {
    size.div_ceil(PART_SIZE)
}

/// The manifest, checked: within limits, and every file as it says.
fn declared(new: &NewCheckpoint, max_bytes: u64) -> Result<Vec<Declared>, ApiError> {
    if new.files.is_empty() || new.files.len() > MAX_FILES {
        return Err(ApiError::invalid(format!("files: 1-{MAX_FILES}")));
    }
    if new.step > i64::MAX as u64 {
        return Err(ApiError::invalid("step: too large"));
    }
    let mut seen = HashSet::new();
    let mut total: u64 = 0;
    let mut out = Vec::with_capacity(new.files.len());
    for (i, f) in new.files.iter().enumerate() {
        let bad = |what: String| ApiError::invalid(format!("file {i} ({:?}): {what}", f.path));
        valid_path(&f.path).map_err(|e| bad(format!("path: {e}")))?;
        if !seen.insert(f.path.as_str()) {
            return Err(bad("listed twice".into()));
        }
        let sha256 = from_hex(&f.sha256)
            .filter(|h| h.len() == 32)
            .ok_or_else(|| bad("sha256: 64 hex digits".into()))?;
        if u64::from(f.parts) != parts_of(f.size) {
            return Err(bad(format!(
                "parts: {} for {} bytes, in parts of {PART_SIZE}",
                parts_of(f.size),
                f.size
            )));
        }
        total = total.saturating_add(f.size);
        out.push(Declared {
            path: f.path.clone(),
            size: f.size as i64,
            sha256,
            compression: f.compression,
            parts: f.parts as i32,
        });
    }
    if total > max_bytes {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            format!("checkpoints are at most {max_bytes} bytes here"),
        ));
    }
    Ok(out)
}

async fn declared_files(state: &AppState, id: Uuid) -> Result<Vec<Declared>, ApiError> {
    let rows: Vec<(String, i64, Vec<u8>, String, i32)> = sqlx::query_as(
        "SELECT path, size, sha256, compression, parts FROM checkpoint_files
         WHERE checkpoint_id = $1 ORDER BY file_index",
    )
    .bind(id)
    .fetch_all(state.pg())
    .await?;
    Ok(rows
        .into_iter()
        .map(|(path, size, sha256, compression, parts)| Declared {
            path,
            size,
            sha256,
            compression: Compression::parse(&compression),
            parts,
        })
        .collect())
}

/// A checkpoint and what's needed to check access: its run and project.
struct Found {
    run: Uuid,
    project: i64,
    state: String,
    prefix: String,
}

async fn find(state: &AppState, user: &AuthUser, id: Uuid, role: Role) -> Result<Found, ApiError> {
    let row: Option<(Uuid, String, String)> =
        sqlx::query_as("SELECT run_id, state, storage_prefix FROM checkpoints WHERE id = $1")
            .bind(id)
            .fetch_optional(state.pg())
            .await?;
    let (run, cstate, prefix) = row.ok_or_else(unknown_checkpoint)?;
    // Someone who can't see the run can't tell the checkpoint exists either.
    let project = auth::require_run(state, user, run, role)
        .await
        .map_err(|err| {
            if err.status == StatusCode::FORBIDDEN && role != Role::Viewer {
                err
            } else {
                unknown_checkpoint()
            }
        })?;
    Ok(Found {
        run,
        project,
        state: cstate,
        prefix,
    })
}

fn part_key(prefix: &str, file: i32, part: i32) -> object_store::path::Path {
    object_store::path::Path::from(format!("{prefix}/{file}/{part}"))
}

/// A checkpoint as the API shows it, with its files if asked.
async fn describe(state: &AppState, id: Uuid, with_files: bool) -> Result<Checkpoint, ApiError> {
    let sql = format!(
        "SELECT c.id, c.run_id AS run, c.step, c.state, c.size,
                COALESCE((SELECT sum(p.size) FROM checkpoint_parts p WHERE p.checkpoint_id = c.id), 0)::bigint
                  AS stored_size,
                (SELECT count(*) FROM checkpoint_files f WHERE f.checkpoint_id = c.id) AS file_count,
                {} AS created_at_ms, {} AS committed_at_ms
         FROM checkpoints c WHERE c.id = $1",
        ms("c.created_at"),
        ms("c.committed_at")
    );
    let mut checkpoint: Checkpoint = sqlx::query_as(&sql)
        .bind(id)
        .fetch_optional(state.pg())
        .await?
        .ok_or_else(unknown_checkpoint)?;
    if with_files {
        let pending = checkpoint.state == "pending";
        let rows: Vec<FileRow> = sqlx::query_as(
            "SELECT f.file_index, f.path, f.size, f.sha256, f.compression, f.parts,
                    COALESCE(sum(p.size), 0)::bigint AS stored_size,
                    COALESCE(array_agg(p.part ORDER BY p.part) FILTER (WHERE p.part IS NOT NULL), '{}')
                      AS uploaded
             FROM checkpoint_files f
             LEFT JOIN checkpoint_parts p
               ON p.checkpoint_id = f.checkpoint_id AND p.file_index = f.file_index
             WHERE f.checkpoint_id = $1
             GROUP BY f.file_index, f.path, f.size, f.sha256, f.compression, f.parts
             ORDER BY f.file_index",
        )
        .bind(id)
        .fetch_all(state.pg())
        .await?;
        checkpoint.files = Some(
            rows.into_iter()
                .map(|f| FileInfo {
                    index: f.file_index,
                    path: f.path,
                    size: f.size,
                    stored_size: f.stored_size,
                    sha256: hex(&f.sha256),
                    compression: Compression::parse(&f.compression),
                    parts: f.parts,
                    uploaded: pending.then_some(f.uploaded),
                })
                .collect(),
        );
    }
    Ok(checkpoint)
}

#[derive(sqlx::FromRow)]
struct FileRow {
    file_index: i32,
    path: String,
    size: i64,
    sha256: Vec<u8>,
    compression: String,
    parts: i32,
    stored_size: i64,
    uploaded: Vec<i32>,
}

fn ms(column: &str) -> String {
    format!("(extract(epoch from {column}) * 1000)::bigint")
}

/// `POST /api/v0/runs/{run}/checkpoints` `{"step", "files": [{"path", "size", "sha256",
/// "compression", "parts"}]}` (editors): a pending checkpoint, with which parts it has. The
/// same manifest again gives back the same checkpoint, to resume an upload; another one at the
/// step of a pending checkpoint replaces it. A committed one has to be deleted first.
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(run): Path<String>,
    Json(new): Json<NewCheckpoint>,
) -> Result<Response, UiError> {
    storage(&state)?;
    let run_id = parse_run_id(&run)?;
    let project_id = auth::require_run(&state, &user, run_id, Role::Editor).await?;
    let files = declared(&new, state.max_checkpoint_bytes())?;
    let step = new.step as i64;

    let mut tx = state.pg().begin().await?;
    let existing: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, state FROM checkpoints WHERE run_id = $1 AND step = $2 FOR UPDATE",
    )
    .bind(run_id)
    .bind(step)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((id, cstate)) = existing {
        if cstate == "committed" {
            return Err(conflict(
                "checkpoint_exists",
                format!("this run has a checkpoint at step {step}: delete it first"),
            )
            .into());
        }
        if declared_files(&state, id).await? == files {
            tx.commit().await?;
            return Ok(Json(describe(&state, id, true).await?).into_response());
        }
        // Another folder at that step: the unfinished upload goes (its files with it).
        sqlx::query("DELETE FROM checkpoints WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    let id = Uuid::now_v7();
    let prefix = format!("{project_id}/{}/{}", run_id.simple(), id.simple());
    let size: i64 = files.iter().map(|f| f.size).sum();
    let inserted = sqlx::query(
        "INSERT INTO checkpoints (id, run_id, step, state, storage_prefix, size, created_by)
         VALUES ($1, $2, $3, 'pending', $4, $5, $6) ON CONFLICT (run_id, step) DO NOTHING",
    )
    .bind(id)
    .bind(run_id)
    .bind(step)
    .bind(&prefix)
    .bind(size)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(conflict(
            "busy",
            format!("a checkpoint at step {step} is being created: try again"),
        )
        .into());
    }
    let indexes: Vec<i32> = (0..files.len() as i32).collect();
    sqlx::query(
        "INSERT INTO checkpoint_files (checkpoint_id, file_index, path, size, sha256, compression, parts)
         SELECT $1, * FROM unnest($2::int[], $3::text[], $4::bigint[], $5::bytea[], $6::text[], $7::int[])",
    )
    .bind(id)
    .bind(&indexes)
    .bind(files.iter().map(|f| f.path.clone()).collect::<Vec<_>>())
    .bind(files.iter().map(|f| f.size).collect::<Vec<_>>())
    .bind(files.iter().map(|f| f.sha256.clone()).collect::<Vec<_>>())
    .bind(files.iter().map(|f| f.compression.as_str()).collect::<Vec<_>>())
    .bind(files.iter().map(|f| f.parts).collect::<Vec<_>>())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    state.clean_soon();
    Ok((StatusCode::CREATED, Json(describe(&state, id, true).await?)).into_response())
}

/// `PUT /api/v0/checkpoints/{id}/files/{file}/parts/{part}` (editors): a part of a file, as
/// declared (compressed or not), with its SHA-256 in `x-losscape-sha256`. Sending one again
/// replaces it.
pub async fn upload_part(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, file, part)): Path<(Uuid, i32, i32)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, UiError> {
    let storage = storage(&state)?;
    let found = find(&state, &user, id, Role::Editor).await?;
    if found.state != "pending" {
        return Err(conflict(
            "committed",
            "this checkpoint is complete: nothing more to upload",
        )
        .into());
    }
    let declared: Option<(i64, String, i32)> = sqlx::query_as(
        "SELECT size, compression, parts FROM checkpoint_files
         WHERE checkpoint_id = $1 AND file_index = $2",
    )
    .bind(id)
    .bind(file)
    .fetch_optional(state.pg())
    .await?;
    let (size, compression, parts) = declared
        .ok_or_else(|| ApiError::invalid(format!("file {file}: not in this checkpoint")))?;
    if part < 0 || part >= parts {
        return Err(ApiError::invalid(format!("part {part}: the file has {parts}")).into());
    }
    // Uncompressed, a part is exactly its share of the file.
    if Compression::parse(&compression) == Compression::None {
        let expected = (size as u64 - part as u64 * PART_SIZE).min(PART_SIZE);
        if body.len() as u64 != expected {
            return Err(ApiError::invalid(format!(
                "part {part}: {} bytes, {expected} expected",
                body.len()
            ))
            .into());
        }
    }
    let sent = headers
        .get(PART_HASH)
        .and_then(|v| v.to_str().ok())
        .and_then(from_hex)
        .filter(|h| h.len() == 32)
        .ok_or_else(|| ApiError::invalid(format!("{PART_HASH}: the part's SHA-256, in hex")))?;
    let hash = Sha256::digest(&body);
    if hash.as_slice() != sent.as_slice() {
        return Err(ApiError::invalid(format!(
            "part {part}: doesn't match its SHA-256 (changed on the way?)"
        ))
        .into());
    }
    let stored = body.len() as i64;
    storage
        .put(&part_key(&found.prefix, file, part), body)
        .await?;
    sqlx::query(
        "INSERT INTO checkpoint_parts (checkpoint_id, file_index, part, size, sha256)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (checkpoint_id, file_index, part) DO UPDATE SET size = $4, sha256 = $5",
    )
    .bind(id)
    .bind(file)
    .bind(part)
    .bind(stored)
    .bind(hash.as_slice())
    .execute(state.pg())
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v0/checkpoints/{id}/commit` (editors): complete, once every part is there.
pub async fn commit(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Checkpoint>, UiError> {
    let found = find(&state, &user, id, Role::Editor).await?;
    if found.state == "pending" {
        let missing: Option<(String, i64)> = sqlx::query_as(
            "SELECT f.path, f.parts - count(p.part)
             FROM checkpoint_files f
             LEFT JOIN checkpoint_parts p
               ON p.checkpoint_id = f.checkpoint_id AND p.file_index = f.file_index
             WHERE f.checkpoint_id = $1
             GROUP BY f.file_index, f.path, f.parts
             HAVING count(p.part) < f.parts
             ORDER BY f.file_index LIMIT 1",
        )
        .bind(id)
        .fetch_optional(state.pg())
        .await?;
        if let Some((path, parts)) = missing {
            return Err(conflict(
                "incomplete",
                format!("{path}: {parts} part(s) not uploaded yet"),
            )
            .into());
        }
        sqlx::query(
            "UPDATE checkpoints SET state = 'committed', committed_at = now()
             WHERE id = $1 AND state = 'pending'",
        )
        .bind(id)
        .execute(state.pg())
        .await?;
        tracing::info!(checkpoint = %id, run = %found.run, project = found.project, "checkpoint committed");
    }
    Ok(Json(describe(&state, id, true).await?))
}

#[derive(Deserialize)]
pub struct ListParams {
    /// Pending ones too (uploads under way or abandoned).
    #[serde(default)]
    pending: bool,
}

/// `GET /api/v0/runs/{run}/checkpoints` (viewers): committed checkpoints, by step.
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(run): Path<String>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<Checkpoint>>, UiError> {
    let run_id = parse_run_id(&run)?;
    auth::require_run(&state, &user, run_id, Role::Viewer).await?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM checkpoints WHERE run_id = $1 AND (state = 'committed' OR $2)
         ORDER BY step",
    )
    .bind(run_id)
    .bind(params.pending)
    .fetch_all(state.pg())
    .await?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(describe(&state, id, false).await?);
    }
    Ok(Json(out))
}

/// `GET /api/v0/checkpoints/{id}` (viewers): with its files.
pub async fn detail(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Checkpoint>, UiError> {
    find(&state, &user, id, Role::Viewer).await?;
    Ok(Json(describe(&state, id, true).await?))
}

#[derive(Deserialize)]
pub struct DownloadParams {
    /// As stored (the SDK decompresses it), with its compression in `x-losscape-compression`.
    /// Otherwise as it was uploaded.
    #[serde(default)]
    stored: bool,
}

/// `GET /api/v0/checkpoints/{id}/files/{file}` (viewers): a file of a committed checkpoint, as a
/// download (never shown in the page, whatever it contains).
pub async fn download(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, file)): Path<(Uuid, i32)>,
    Query(params): Query<DownloadParams>,
) -> Result<Response, UiError> {
    let storage = storage(&state)?;
    let found = find(&state, &user, id, Role::Viewer).await?;
    if found.state != "committed" {
        return Err(conflict("pending", "this checkpoint is still being uploaded").into());
    }
    let row: Option<(String, i64, Vec<u8>, String, i32)> = sqlx::query_as(
        "SELECT path, size, sha256, compression, parts FROM checkpoint_files
         WHERE checkpoint_id = $1 AND file_index = $2",
    )
    .bind(id)
    .bind(file)
    .fetch_optional(state.pg())
    .await?;
    let (path, size, sha256, compression, parts) =
        row.ok_or_else(|| ApiError::invalid(format!("file {file}: not in this checkpoint")))?;
    let keys = (0..parts)
        .map(|p| part_key(&found.prefix, file, p))
        .collect();
    let stream = storage.read(keys);
    let compression = Compression::parse(&compression);

    let (body, length) = if params.stored || compression == Compression::None {
        let stored: i64 = sqlx::query_scalar(
            "SELECT COALESCE(sum(size), 0)::bigint FROM checkpoint_parts
             WHERE checkpoint_id = $1 AND file_index = $2",
        )
        .bind(id)
        .bind(file)
        .fetch_one(state.pg())
        .await?;
        (Body::from_stream(stream), stored)
    } else {
        // Each part is a zstd frame: together, one zstd stream.
        let reader = tokio_util::io::StreamReader::new(stream);
        let mut decoder = async_compression::tokio::bufread::ZstdDecoder::new(reader);
        decoder.multiple_members(true);
        (
            Body::from_stream(tokio_util::io::ReaderStream::new(decoder)),
            size,
        )
    };
    let name = download_name(&path);
    let mut response = (
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
            (header::CONTENT_LENGTH, length.to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        body,
    )
        .into_response();
    let headers = response.headers_mut();
    headers.insert("x-losscape-sha256", hex(&sha256).parse().expect("hex"));
    if params.stored {
        headers.insert(
            "x-losscape-compression",
            compression.as_str().parse().expect("ascii"),
        );
    }
    Ok(response)
}

/// The file's name, without anything a header could misread.
fn download_name(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    let mut name: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    name.truncate(100);
    if name.trim_matches(['.', '_']).is_empty() {
        "file".into()
    } else {
        name
    }
}

/// `DELETE /api/v0/checkpoints/{id}` (editors): the checkpoint, and its files from storage
/// shortly after.
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, UiError> {
    let found = find(&state, &user, id, Role::Editor).await?;
    sqlx::query("DELETE FROM checkpoints WHERE id = $1")
        .bind(id)
        .execute(state.pg())
        .await?;
    tracing::info!(checkpoint = %id, run = %found.run, user = user.id, "checkpoint deleted");
    state.clean_soon();
    Ok(StatusCode::NO_CONTENT)
}

/// Every minute, or sooner when asked: pending checkpoints left too long go, and the files of
/// deleted ones are deleted from storage.
pub fn spawn_cleaner(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(err) = expire_pending(&state, PENDING_HOURS).await {
                tracing::warn!(%err.message, "expiring pending checkpoints failed");
            }
            match collect_garbage(&state).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(objects = n, "deleted files of deleted checkpoints"),
                Err(err) => tracing::warn!(%err.message, "deleting checkpoint files failed"),
            }
            tokio::select! {
                _ = state.clean_requested() => {}
                _ = tokio::time::sleep(CLEAN_EVERY) => {}
            }
        }
    })
}

/// Deletes pending checkpoints created more than `hours` ago. Returns how many.
pub async fn expire_pending(state: &AppState, hours: i32) -> Result<u64, ApiError> {
    Ok(sqlx::query(
        "DELETE FROM checkpoints
         WHERE state = 'pending' AND created_at < now() - make_interval(hours => $1)",
    )
    .bind(hours)
    .execute(state.pg())
    .await?
    .rows_affected())
}

/// Deletes from storage what deleted checkpoints left. Returns how many objects. Without
/// storage, it stays queued.
pub async fn collect_garbage(state: &AppState) -> Result<u64, ApiError> {
    let Some(storage) = state.storage() else {
        return Ok(0);
    };
    let mut total = 0;
    loop {
        let prefixes: Vec<String> =
            sqlx::query_scalar("SELECT prefix FROM storage_garbage ORDER BY queued_at LIMIT 100")
                .fetch_all(state.pg())
                .await?;
        if prefixes.is_empty() {
            return Ok(total);
        }
        for prefix in prefixes {
            total += storage
                .delete_prefix(&object_store::path::Path::from(prefix.as_str()))
                .await?;
            sqlx::query("DELETE FROM storage_garbage WHERE prefix = $1")
                .bind(&prefix)
                .execute(state.pg())
                .await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, size: u64) -> NewFile {
        NewFile {
            path: path.into(),
            size,
            sha256: "ab".repeat(32),
            compression: Compression::None,
            parts: parts_of(size) as u32,
        }
    }

    #[test]
    fn paths_mean_the_same_everywhere() {
        for good in [
            "model.pt",
            "shards/0001.safetensors",
            "a b/ü.json",
            ".hidden",
        ] {
            assert!(valid_path(good).is_ok(), "{good}");
        }
        let long = "x".repeat(256);
        for bad in [
            "",
            "/abs",
            "a//b",
            "a/./b",
            "../up",
            "a/..",
            "a\\b",
            "tab\there",
            "nul\0",
            long.as_str(),
        ] {
            assert!(valid_path(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn manifests_are_checked() {
        let ok = NewCheckpoint {
            step: 10,
            files: vec![file("model.pt", PART_SIZE * 2 + 1), file("empty", 0)],
        };
        let files = declared(&ok, u64::MAX).unwrap();
        assert_eq!(files[0].parts, 3);
        assert_eq!(files[1].parts, 0, "an empty file has no parts");

        let mut wrong_parts = file("a", 10);
        wrong_parts.parts = 2;
        let twice = NewCheckpoint {
            step: 0,
            files: vec![file("a", 1), file("a", 1)],
        };
        let mut bad_hash = file("a", 1);
        bad_hash.sha256 = "xyz".into();
        for (bad, why) in [
            (
                NewCheckpoint {
                    step: 0,
                    files: vec![],
                },
                "no files",
            ),
            (
                NewCheckpoint {
                    step: 0,
                    files: vec![wrong_parts],
                },
                "parts",
            ),
            (twice, "a path twice"),
            (
                NewCheckpoint {
                    step: 0,
                    files: vec![bad_hash],
                },
                "hash",
            ),
            (
                NewCheckpoint {
                    step: 0,
                    files: vec![file("../x", 1)],
                },
                "path",
            ),
        ] {
            assert!(declared(&bad, u64::MAX).is_err(), "{why}");
        }
        let big = NewCheckpoint {
            step: 0,
            files: vec![file("a", 100)],
        };
        assert_eq!(
            declared(&big, 99).unwrap_err().status,
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    #[test]
    fn download_names_are_plain() {
        assert_eq!(
            download_name("shards/model-00001.safetensors"),
            "model-00001.safetensors"
        );
        assert_eq!(download_name("a\"; evil=1"), "a___evil_1");
        assert_eq!(download_name("..."), "file");
        assert_eq!(hex(&from_hex("00ff10").unwrap()), "00ff10");
        assert!(from_hex("0g").is_none());
    }
}
