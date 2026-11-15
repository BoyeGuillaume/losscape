//! Checkpoints: folders a run uploads at a step, kept in the configured storage
//! (docs/ARTIFACTS.md). One is created from its manifest, its files are uploaded in parts (each
//! checked against its hash), and it's committed once they're all there. All of it goes through
//! the server, which checks who asks: storage is never reached directly.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use futures_util::stream::{self, BoxStream};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::{self, AuthUser, Role};
use crate::error::ApiError;
use crate::hashing;
use crate::read::{UiError, parse_run_id};
use crate::retention::{self, Keep, ValueAt};
use crate::state::AppState;
use crate::storage::Storage;
use crate::tar;

/// Bytes of a file per part, before compression.
pub const PART_SIZE: u64 = losscape_core::CHECKPOINT_PART_SIZE;
/// Most a part can take stored: zstd's worst case on data it can't compress, and some room.
pub const MAX_STORED_PART: usize = PART_SIZE as usize + PART_SIZE as usize / 64 + 64 * 1024;
pub const MAX_FILES: usize = 10_000;
const MAX_PATH_BYTES: usize = 1024;
const MAX_NAME_BYTES: usize = 255;
/// Pending checkpoints never committed go after this long, with what was uploaded.
const PENDING_HOURS: i32 = 24;
const CLEAN_EVERY: Duration = Duration::from_secs(60);
/// Header with the SHA-256 of a part, as sent (hex).
const PART_HASH: &str = losscape_core::PART_HASH_HEADER;

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
    /// BLAKE3 of the file as it is, before compression (hex): what the server computes from
    /// the parts must match.
    blake3: String,
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
    /// Never deleted by retention.
    important: bool,
    /// Why retention keeps it: important, and with retention on, its run's last or best one.
    #[sqlx(skip)]
    keep: Vec<Keep>,
    /// When retention deletes it, if it does.
    #[sqlx(skip)]
    expires_at_ms: Option<i64>,
    /// Its link, by name: `/ckpt/<project>/<run>:<step>`, all of it as a tar archive;
    /// `<url>/<path>` for one file. Until listed with its run: `permanent_url`.
    #[sqlx(skip)]
    url: String,
    /// `/ckpt/<id>`: always this checkpoint.
    #[sqlx(skip)]
    permanent_url: String,
    /// The run's best checkpoint, by the project's metric for it.
    #[sqlx(skip)]
    best: bool,
    /// BLAKE3 of its files listed as `b3sum` lists them (`<hash>  <path>`, by path): changes with
    /// any of them. Computed by the server at commit, from the hashes it computed itself.
    digest: Option<String>,
    /// What storage gave back that didn't match what was uploaded, if it ever did.
    corrupt: Option<String>,
    /// Latest values of the metrics asked for, at its step.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[sqlx(skip)]
    metrics: Option<BTreeMap<String, ValueAt>>,
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
    /// BLAKE3 of its content (hex), as `b3sum` gives it.
    blake3: String,
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
    blake3: Vec<u8>,
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

/// Reading checkpoints takes the viewer role: guests read runs and charts, not checkpoints.
pub(crate) fn checkpoint_access(err: ApiError, min: Role) -> ApiError {
    if err.status == StatusCode::FORBIDDEN && min == Role::Viewer {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "your role in this project doesn't include checkpoints: it takes viewer or more",
        )
    } else {
        err
    }
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
        let blake3 = from_hex(&f.blake3)
            .filter(|h| h.len() == 32)
            .ok_or_else(|| bad("blake3: 64 hex digits".into()))?;
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
            blake3,
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
        "SELECT path, size, blake3, compression, parts FROM checkpoint_files
         WHERE checkpoint_id = $1 ORDER BY file_index",
    )
    .bind(id)
    .fetch_all(state.pg())
    .await?;
    Ok(rows
        .into_iter()
        .map(|(path, size, blake3, compression, parts)| Declared {
            path,
            size,
            blake3,
            compression: Compression::parse(&compression),
            parts,
        })
        .collect())
}

/// A checkpoint and what's needed to check access: its run and project.
struct Found {
    id: Uuid,
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
    // Someone who can't see the run can't tell the checkpoint exists either; a member whose
    // role doesn't include it is told so.
    let project = auth::require_run(state, user, run, role)
        .await
        .map_err(|err| {
            if err.status == StatusCode::FORBIDDEN {
                checkpoint_access(err, role)
            } else {
                unknown_checkpoint()
            }
        })?;
    Ok(Found {
        id,
        run,
        project,
        state: cstate,
        prefix,
    })
}

/// Where a part is in storage: `<checkpoint's place>/<path>.<part>`, `.zst` added when
/// compressed (files stored before names were kept: `<index>/<part>`).
fn part_key(
    prefix: &str,
    name: Option<&str>,
    file: i32,
    part: i32,
    compression: Compression,
) -> object_store::path::Path {
    let key = match name {
        Some(name) => {
            let zst = if compression == Compression::Zstd {
                ".zst"
            } else {
                ""
            };
            format!("{prefix}/{name}.{part:05}{zst}")
        }
        None => format!("{prefix}/{file}/{part}"),
    };
    object_store::path::Path::from(key)
}

/// A checkpoint's place in storage, readable when browsing it (an S3 console):
/// `<project>/<run name>-<run id end>/step-<step>-<checkpoint id end>`. The ids' random ends
/// keep two runs of one name apart, and a checkpoint from one deleted at its step (whose files
/// may still be going).
fn storage_prefix(project: &str, run_name: &str, run: Uuid, step: u64, id: Uuid) -> String {
    let tail = |id: Uuid| id.simple().to_string()[24..].to_owned();
    format!(
        "{}/{}-{}/step-{step:010}-{}",
        key_part(project),
        key_part(run_name),
        tail(run),
        tail(id)
    )
}

/// A name as a part of a storage key: letters, digits, `.`, `_` and `-`; the rest `_`.
fn key_part(name: &str) -> String {
    let mut part: String = name
        .chars()
        .take(100)
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    if part.is_empty() || part.starts_with('.') {
        part.insert(0, '_');
    }
    part
}

/// Columns of a checkpoint as the API shows it, from `checkpoints c`.
fn columns() -> String {
    format!(
        "c.id, c.run_id AS run, c.step, c.state, c.size,
         COALESCE((SELECT sum(p.size) FROM checkpoint_parts p WHERE p.checkpoint_id = c.id), 0)::bigint
           AS stored_size,
         (SELECT count(*) FROM checkpoint_files f WHERE f.checkpoint_id = c.id) AS file_count,
         {} AS created_at_ms, {} AS committed_at_ms, c.important,
         encode(c.digest, 'hex') AS digest, c.corrupt",
        ms("c.created_at"),
        ms("c.committed_at")
    )
}

/// A checkpoint as the API shows it, with its files if asked.
async fn describe(state: &AppState, id: Uuid, with_files: bool) -> Result<Checkpoint, ApiError> {
    let sql = format!("SELECT {} FROM checkpoints c WHERE c.id = $1", columns());
    let mut checkpoint: Checkpoint = sqlx::query_as(&sql)
        .bind(id)
        .fetch_optional(state.pg())
        .await?
        .ok_or_else(unknown_checkpoint)?;
    checkpoint.permanent_url = format!("/ckpt/{}", checkpoint.id);
    checkpoint.url = checkpoint.permanent_url.clone();
    if with_files {
        let pending = checkpoint.state == "pending";
        let rows: Vec<FileRow> = sqlx::query_as(
            "SELECT f.file_index, f.path, f.size, f.blake3, f.compression, f.parts,
                    COALESCE(sum(p.size), 0)::bigint AS stored_size,
                    COALESCE(array_agg(p.part ORDER BY p.part) FILTER (WHERE p.part IS NOT NULL), '{}')
                      AS uploaded
             FROM checkpoint_files f
             LEFT JOIN checkpoint_parts p
               ON p.checkpoint_id = f.checkpoint_id AND p.file_index = f.file_index
             WHERE f.checkpoint_id = $1
             GROUP BY f.file_index, f.path, f.size, f.blake3, f.compression, f.parts
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
                    blake3: hex(&f.blake3),
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
    blake3: Vec<u8>,
    compression: String,
    parts: i32,
    stored_size: i64,
    uploaded: Vec<i32>,
}

fn ms(column: &str) -> String {
    format!("(extract(epoch from {column}) * 1000)::bigint")
}

/// `POST /api/v0/runs/{run}/checkpoints` `{"step", "files": [{"path", "size", "blake3",
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
    auth::require_run(&state, &user, run_id, Role::Editor).await?;
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
    let (project_name, run_name): (String, String) = sqlx::query_as(
        "SELECT p.name, r.name FROM runs r JOIN projects p ON p.id = r.project_id WHERE r.id = $1",
    )
    .bind(run_id)
    .fetch_one(&mut *tx)
    .await?;
    let prefix = storage_prefix(&project_name, &run_name, run_id, step as u64, id);
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
        "INSERT INTO checkpoint_files
           (checkpoint_id, file_index, path, size, blake3, compression, parts, storage_name)
         SELECT $1, *, path FROM unnest($2::int[], $3::text[], $4::bigint[], $5::bytea[],
                                        $6::text[], $7::int[]) AS f(file_index, path)",
    )
    .bind(id)
    .bind(&indexes)
    .bind(files.iter().map(|f| f.path.clone()).collect::<Vec<_>>())
    .bind(files.iter().map(|f| f.size).collect::<Vec<_>>())
    .bind(files.iter().map(|f| f.blake3.clone()).collect::<Vec<_>>())
    .bind(
        files
            .iter()
            .map(|f| f.compression.as_str())
            .collect::<Vec<_>>(),
    )
    .bind(files.iter().map(|f| f.parts).collect::<Vec<_>>())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    state.clean_soon();
    Ok((StatusCode::CREATED, Json(describe(&state, id, true).await?)).into_response())
}

/// `PUT /api/v0/checkpoints/{id}/files/{file}/parts/{part}` (editors): a part of a file, as
/// declared (compressed or not), with its BLAKE3 in `x-losscape-blake3`. Sending one again
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
    let declared: Option<(i64, String, i32, Option<String>)> = sqlx::query_as(
        "SELECT size, compression, parts, storage_name FROM checkpoint_files
         WHERE checkpoint_id = $1 AND file_index = $2",
    )
    .bind(id)
    .bind(file)
    .fetch_optional(state.pg())
    .await?;
    let (size, compression, parts, name) = declared
        .ok_or_else(|| ApiError::invalid(format!("file {file}: not in this checkpoint")))?;
    if part < 0 || part >= parts {
        return Err(ApiError::invalid(format!("part {part}: the file has {parts}")).into());
    }
    let compression = Compression::parse(&compression);
    let sent = headers
        .get(PART_HASH)
        .and_then(|v| v.to_str().ok())
        .and_then(from_hex)
        .filter(|h| h.len() == 32)
        .ok_or_else(|| ApiError::invalid(format!("{PART_HASH}: the part's BLAKE3, in hex")))?;
    let stored_hash = blake3::hash(&body);
    if stored_hash.as_bytes() != sent.as_slice() {
        return Err(ApiError::invalid(format!(
            "part {part}: doesn't match its BLAKE3 (changed on the way?)"
        ))
        .into());
    }
    // Its content, decompressed: exactly its share of the file, hashed for its place in it.
    let expected = (size as u64 - part as u64 * PART_SIZE).min(PART_SIZE);
    let content_hash = {
        let body = body.clone();
        tokio::task::spawn_blocking(move || -> Result<hashing::Hash, String> {
            let decompressed;
            let content: &[u8] = match compression {
                Compression::None => &body,
                Compression::Zstd => {
                    // At most a part's length: nothing can make it decompress to more.
                    decompressed = zstd::bulk::decompress(&body, PART_SIZE as usize)
                        .map_err(|e| format!("not zstd, or more than a part: {e}"))?;
                    &decompressed
                }
            };
            if content.len() as u64 != expected {
                return Err(format!(
                    "{} bytes of the file, {expected} expected",
                    content.len()
                ));
            }
            Ok(hashing::part_hash(
                content,
                part as u64,
                parts as u64,
                PART_SIZE,
            ))
        })
        .await
        .expect("hashing doesn't panic")
        .map_err(|e| ApiError::invalid(format!("part {part}: {e}")))?
    };
    let stored = body.len() as i64;
    storage
        .put(
            &part_key(&found.prefix, name.as_deref(), file, part, compression),
            body,
        )
        .await?;
    sqlx::query(
        "INSERT INTO checkpoint_parts (checkpoint_id, file_index, part, size, blake3, content)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (checkpoint_id, file_index, part)
         DO UPDATE SET size = $4, blake3 = $5, content = $6",
    )
    .bind(id)
    .bind(file)
    .bind(part)
    .bind(stored)
    .bind(stored_hash.as_bytes().as_slice())
    .bind(content_hash.as_slice())
    .execute(state.pg())
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A file being committed: what was declared, and what each part contributes to its hash.
#[derive(sqlx::FromRow)]
struct CommittedFile {
    path: String,
    size: i64,
    declared: Vec<u8>,
    parts: Vec<Vec<u8>>,
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
        // Each file's BLAKE3 from its parts' (computed as they arrived): what was declared, or the
        // file changed while it was read for the upload.
        let rows: Vec<CommittedFile> = sqlx::query_as(
            "SELECT f.path, f.size, f.blake3 AS declared,
                    COALESCE(array_agg(p.content ORDER BY p.part)
                             FILTER (WHERE p.part IS NOT NULL), '{}') AS parts
             FROM checkpoint_files f
             LEFT JOIN checkpoint_parts p
               ON p.checkpoint_id = f.checkpoint_id AND p.file_index = f.file_index
             WHERE f.checkpoint_id = $1
             GROUP BY f.file_index, f.path, f.size, f.blake3 ORDER BY f.file_index",
        )
        .bind(id)
        .fetch_all(state.pg())
        .await?;
        let mut listing = Vec::with_capacity(rows.len());
        for CommittedFile {
            path,
            size,
            declared,
            parts,
        } in rows
        {
            let parts: Vec<hashing::Hash> = parts
                .iter()
                .map(|p| p.as_slice().try_into().expect("32 bytes, as stored"))
                .collect();
            let actual = hashing::file_hash(&parts, size as u64, PART_SIZE);
            if actual.as_slice() != declared.as_slice() {
                return Err(ApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "hash_mismatch",
                    format!(
                        "{path}: its content has BLAKE3 {}, not the {} declared (changed while it was read?)",
                        hex(&actual),
                        hex(&declared)
                    ),
                )
                .into());
            }
            listing.push((path, actual));
        }
        let digest = hashing::digest(listing.iter().map(|(p, h)| (p.as_str(), &h[..])));
        sqlx::query(
            "UPDATE checkpoints SET state = 'committed', committed_at = now(), digest = $2
             WHERE id = $1 AND state = 'pending'",
        )
        .bind(id)
        .bind(digest.as_slice())
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
    /// Only the pending ones: what changes while a run uploads, cheap to follow.
    #[serde(default)]
    pending_only: bool,
    /// Metrics to give the latest values of at each checkpoint's step (repeated).
    #[serde(default)]
    metric: Vec<String>,
}

/// `GET /api/v0/runs/{run}/checkpoints[?pending=true | ?pending_only=true][&metric=…]`
/// (viewers): committed checkpoints by step, with what retention does with them.
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(run): Path<String>,
    axum_extra::extract::Query(params): axum_extra::extract::Query<ListParams>,
) -> Result<Json<Vec<Checkpoint>>, UiError> {
    let run_id = parse_run_id(&run)?;
    let project_id = auth::require_run(&state, &user, run_id, Role::Viewer)
        .await
        .map_err(|e| checkpoint_access(e, Role::Viewer))?;
    if params.metric.len() > MAX_METRICS {
        return Err(ApiError::invalid(format!("at most {MAX_METRICS} metrics")).into());
    }
    // One query, however many there are (a run can have thousands).
    let mut out: Vec<Checkpoint> = sqlx::query_as(&format!(
        "SELECT {} FROM checkpoints c
         WHERE c.run_id = $1
           AND CASE WHEN $3 THEN c.state = 'pending' ELSE c.state = 'committed' OR $2 END
         ORDER BY c.step",
        columns()
    ))
    .bind(run_id)
    .bind(params.pending)
    .bind(params.pending_only)
    .fetch_all(state.pg())
    .await?;
    for c in &mut out {
        c.permanent_url = format!("/ckpt/{}", c.id);
        c.url = c.permanent_url.clone();
    }
    if !params.pending_only {
        annotate(&state, project_id, run_id, &mut out, &params.metric).await?;
    }
    Ok(Json(out))
}

/// A run's committed checkpoints in brief, as columns (one entry per checkpoint, by step).
#[derive(Serialize, Default)]
pub struct Brief {
    step: Vec<i64>,
    committed_at_ms: Vec<i64>,
    size: Vec<i64>,
    file_count: Vec<i64>,
    important: Vec<bool>,
}

/// `GET /api/v0/runs/{run}/checkpoints/brief` (viewers): every committed checkpoint of a run
/// in brief, for marking charts: small however many there are.
pub async fn brief(
    State(state): State<AppState>,
    user: AuthUser,
    Path(run): Path<String>,
) -> Result<Json<Brief>, UiError> {
    let run_id = parse_run_id(&run)?;
    auth::require_run(&state, &user, run_id, Role::Viewer)
        .await
        .map_err(|e| checkpoint_access(e, Role::Viewer))?;
    let rows: Vec<(i64, i64, i64, i64, bool)> = sqlx::query_as(&format!(
        "SELECT c.step, {}, c.size,
                (SELECT count(*) FROM checkpoint_files f WHERE f.checkpoint_id = c.id), c.important
         FROM checkpoints c WHERE c.run_id = $1 AND c.state = 'committed' ORDER BY c.step",
        ms("COALESCE(c.committed_at, c.created_at)")
    ))
    .bind(run_id)
    .fetch_all(state.pg())
    .await?;
    let mut out = Brief::default();
    for (step, at, size, files, important) in rows {
        out.step.push(step);
        out.committed_at_ms.push(at);
        out.size.push(size);
        out.file_count.push(files);
        out.important.push(important);
    }
    Ok(Json(out))
}

/// `GET /api/v0/checkpoints/{id}` (viewers): with its files.
pub async fn detail(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Checkpoint>, UiError> {
    Ok(Json(detail_of(&state, &user, id).await?))
}

/// A checkpoint with its files, as `GET /api/v0/checkpoints/{id}` gives it.
pub(crate) async fn detail_of(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
) -> Result<Checkpoint, ApiError> {
    let found = find(state, user, id, Role::Viewer).await?;
    let mut checkpoint = describe(state, id, true).await?;
    annotate(
        state,
        found.project,
        found.run,
        std::slice::from_mut(&mut checkpoint),
        &[],
    )
    .await?;
    Ok(checkpoint)
}

#[derive(Deserialize)]
pub struct CheckpointUpdate {
    important: Option<bool>,
}

/// `PATCH /api/v0/checkpoints/{id}` `{"important": bool}` (editors): important checkpoints are
/// never deleted by retention.
pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<CheckpointUpdate>,
) -> Result<Json<Checkpoint>, UiError> {
    let found = find(&state, &user, id, Role::Editor).await?;
    if let Some(important) = body.important {
        sqlx::query("UPDATE checkpoints SET important = $2 WHERE id = $1")
            .bind(id)
            .bind(important)
            .execute(state.pg())
            .await?;
    }
    let mut checkpoint = describe(&state, id, false).await?;
    annotate(
        &state,
        found.project,
        found.run,
        std::slice::from_mut(&mut checkpoint),
        &[],
    )
    .await?;
    Ok(Json(checkpoint))
}

#[derive(Deserialize)]
pub struct Prune {
    /// Keep about this many, spaced regularly by step (none: keep none for spacing).
    spaced: Option<u32>,
    /// Keep the run's last one.
    #[serde(default)]
    last: bool,
    /// Keep the run's best one by a metric.
    best: Option<retention::Best>,
    /// Only say what it would delete.
    #[serde(default)]
    dry_run: bool,
}

#[derive(Serialize)]
pub struct Pruned {
    deleted: u64,
    deleted_bytes: i64,
    kept: u64,
    dry_run: bool,
}

/// `POST /api/v0/runs/{run}/checkpoints/prune` `{"spaced": n, "last", "best": {"metric",
/// "goal"}, "dry_run"}` (editors): deletes a run's checkpoints but about `n` spaced regularly,
/// the last and the best if asked, and always the important ones. With `dry_run`, only counts.
pub async fn prune(
    State(state): State<AppState>,
    user: AuthUser,
    Path(run): Path<String>,
    Json(body): Json<Prune>,
) -> Result<Json<Pruned>, UiError> {
    let run_id = parse_run_id(&run)?;
    let project_id = auth::require_run(&state, &user, run_id, Role::Editor).await?;
    let policy = retention::Retention {
        days: None,
        keep_last: body.last,
        keep_best: body.best.is_some(),
        best: body.best,
    };
    policy.check()?;
    let rows: Vec<(Uuid, i64, bool, i64)> = sqlx::query_as(
        "SELECT id, step, important, size FROM checkpoints
         WHERE run_id = $1 AND state = 'committed'",
    )
    .bind(run_id)
    .fetch_all(state.pg())
    .await?;
    let stored: Vec<retention::Stored> = rows
        .iter()
        .map(|&(id, step, important, _)| retention::Stored {
            id,
            step,
            important,
        })
        .collect();
    let protected = retention::protected(&state, project_id, run_id, &policy, &stored).await?;
    let steps: Vec<i64> = rows.iter().map(|r| r.1).collect();
    let spaced = retention::spaced(&steps, body.spaced.unwrap_or(0) as usize);
    let (gone, kept): (Vec<&(Uuid, i64, bool, i64)>, Vec<_>) =
        rows.iter().partition(|(id, step, important, _)| {
            !important && !protected.contains_key(id) && !spaced.contains(step)
        });
    let deleted_bytes = gone.iter().map(|r| r.3).sum();
    let mut deleted = gone.len() as u64;
    if !body.dry_run && !gone.is_empty() {
        let ids: Vec<Uuid> = gone.iter().map(|r| r.0).collect();
        // Not one marked important since it was read.
        deleted = sqlx::query(
            "DELETE FROM checkpoints WHERE run_id = $1 AND id = ANY($2) AND NOT important",
        )
        .bind(run_id)
        .bind(&ids)
        .execute(state.pg())
        .await?
        .rows_affected();
        tracing::info!(run = %run_id, deleted, kept = kept.len(), user = user.id, "checkpoints pruned");
        state.clean_soon();
    }
    Ok(Json(Pruned {
        deleted,
        deleted_bytes,
        kept: kept.len() as u64,
        dry_run: body.dry_run,
    }))
}

#[derive(Deserialize)]
pub struct MarkAll {
    important: bool,
}

/// `POST /api/v0/runs/{run}/checkpoints/important` `{"important": bool}` (editors): marks every
/// committed checkpoint of a run important, or none. Returns how many changed.
pub async fn mark_all(
    State(state): State<AppState>,
    user: AuthUser,
    Path(run): Path<String>,
    Json(body): Json<MarkAll>,
) -> Result<Json<serde_json::Value>, UiError> {
    let run_id = parse_run_id(&run)?;
    auth::require_run(&state, &user, run_id, Role::Editor).await?;
    let changed = sqlx::query(
        "UPDATE checkpoints SET important = $2
         WHERE run_id = $1 AND state = 'committed' AND important <> $2",
    )
    .bind(run_id)
    .bind(body.important)
    .execute(state.pg())
    .await?
    .rows_affected();
    Ok(Json(serde_json::json!({ "changed": changed })))
}

/// Metrics whose values a listing gives at each checkpoint's step, at most.
const MAX_METRICS: usize = 20;

/// Fills in what retention does with committed checkpoints of a run, and the latest values of
/// `metrics` at their steps.
async fn annotate(
    state: &AppState,
    project_id: i64,
    run_id: Uuid,
    checkpoints: &mut [Checkpoint],
    metrics: &[String],
) -> Result<(), ApiError> {
    let policy = retention::load(state, project_id).await?;
    let keep = match policy.days {
        Some(_) => {
            let stored = retention::stored(state, run_id).await?;
            retention::protected(state, project_id, run_id, &policy, &stored).await?
        }
        None => HashMap::new(),
    };
    let best = match &policy.best {
        Some(best) => {
            let stored = retention::stored(state, run_id).await?;
            retention::best_of(state, project_id, run_id, best, &stored).await?
        }
        None => None,
    };
    let link = crate::links::run_link(state, run_id).await?;
    let steps: Vec<u64> = checkpoints.iter().map(|c| c.step as u64).collect();
    let values = retention::values_at(state, project_id, run_id, &steps, metrics).await?;
    for c in checkpoints.iter_mut().filter(|c| c.state == "committed") {
        c.url = format!("{link}:{}", c.step);
        c.best = best == Some(c.id);
        c.keep = match keep.get(&c.id) {
            Some(reasons) => reasons.clone(),
            None if c.important => vec![Keep::Important],
            None => Vec::new(),
        };
        if let Some(days) = policy.days
            && c.keep.is_empty()
        {
            c.expires_at_ms = c.committed_at_ms.map(|t| t + i64::from(days) * 86_400_000);
        }
        if !metrics.is_empty() {
            c.metrics = Some(values.get(&(c.step as u64)).cloned().unwrap_or_default());
        }
    }
    Ok(())
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
    request: HeaderMap,
) -> Result<Response, UiError> {
    let found = committed(&state, &user, id).await?;
    let row: Option<StoredFile> = sqlx::query_as(&format!(
        "SELECT {FILE_COLUMNS} FROM checkpoint_files WHERE checkpoint_id = $1 AND file_index = $2"
    ))
    .bind(id)
    .bind(file)
    .fetch_optional(state.pg())
    .await?;
    let file =
        row.ok_or_else(|| ApiError::invalid(format!("file {file}: not in this checkpoint")))?;
    serve_file(&state, &found, file, params.stored, &request, true).await
}

/// A file of a checkpoint by its path. `fixed`: asked for by the checkpoint's id, so it never
/// changes (cached for good); by a name or `:latest`, it can be another checkpoint later.
pub(crate) async fn serve_path(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    path: &str,
    request: &HeaderMap,
    fixed: bool,
) -> Result<Response, UiError> {
    let found = committed(state, user, id).await?;
    let row: Option<StoredFile> = sqlx::query_as(&format!(
        "SELECT {FILE_COLUMNS} FROM checkpoint_files WHERE checkpoint_id = $1 AND path = $2"
    ))
    .bind(id)
    .bind(path)
    .fetch_optional(state.pg())
    .await?;
    let file = row.ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown_file",
            format!("{path}: not in this checkpoint"),
        )
    })?;
    serve_file(state, &found, file, false, request, fixed).await
}

/// A committed checkpoint the user can read.
async fn committed(state: &AppState, user: &AuthUser, id: Uuid) -> Result<Found, ApiError> {
    let found = find(state, user, id, Role::Viewer).await?;
    if found.state != "committed" {
        return Err(conflict(
            "pending",
            "this checkpoint is still being uploaded",
        ));
    }
    Ok(found)
}

const FILE_COLUMNS: &str = "file_index, path, size, blake3, compression, parts, storage_name,
    (SELECT COALESCE(array_agg(p.blake3 ORDER BY p.part), '{}') FROM checkpoint_parts p
     WHERE p.checkpoint_id = checkpoint_files.checkpoint_id
       AND p.file_index = checkpoint_files.file_index) AS part_hashes";

#[derive(sqlx::FromRow)]
struct StoredFile {
    file_index: i32,
    path: String,
    size: i64,
    blake3: Vec<u8>,
    compression: String,
    parts: i32,
    storage_name: Option<String>,
    /// BLAKE3 of each part as stored, by part: what storage must give back.
    part_hashes: Vec<Vec<u8>>,
}

impl StoredFile {
    /// Its parts from `first` on, with what each must hash to.
    fn checked_keys(&self, prefix: &str, first: i32) -> Vec<(object_store::path::Path, [u8; 32])> {
        (first..self.parts)
            .map(|p| {
                let hash = self
                    .part_hashes
                    .get(p as usize)
                    .and_then(|h| <[u8; 32]>::try_from(h.as_slice()).ok())
                    // Not there: nothing matches it, so reading it fails rather than trusts.
                    .unwrap_or_default();
                (self.key(prefix, p), hash)
            })
            .collect()
    }

    fn key(&self, prefix: &str, part: i32) -> object_store::path::Path {
        let compression = Compression::parse(&self.compression);
        part_key(
            prefix,
            self.storage_name.as_deref(),
            self.file_index,
            part,
            compression,
        )
    }
}

/// A file as a download: as uploaded (with `Range`, `ETag` and `If-None-Match`), or as stored.
async fn serve_file(
    state: &AppState,
    found: &Found,
    file: StoredFile,
    stored: bool,
    request: &HeaderMap,
    fixed: bool,
) -> Result<Response, UiError> {
    let storage = storage(state)?;
    let compression = Compression::parse(&file.compression);
    let sha = hex(&file.blake3);
    // Its content never changes: a checkpoint is never modified, only deleted.
    let etag = format!("\"{sha}\"");
    let mut headers = HeaderMap::new();
    let mut set = |name: header::HeaderName, value: String| {
        headers.insert(name, value.parse().expect("header value"));
    };
    set(header::CONTENT_TYPE, "application/octet-stream".into());
    set(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{}\"", download_name(&file.path)),
    );
    set(header::X_CONTENT_TYPE_OPTIONS, "nosniff".into());
    set(header::HeaderName::from_static(PART_HASH), sha);

    let corrupt = report_corrupt(state, found.id);
    if stored {
        let keys = file.checked_keys(&found.prefix, 0);
        let length: i64 = sqlx::query_scalar(
            "SELECT COALESCE(sum(size), 0)::bigint FROM checkpoint_parts
             WHERE checkpoint_id = $1 AND file_index = $2",
        )
        .bind(found.id)
        .bind(file.file_index)
        .fetch_one(state.pg())
        .await?;
        set(header::CONTENT_LENGTH, length.to_string());
        set(
            header::HeaderName::from_static("x-losscape-compression"),
            compression.as_str().into(),
        );
        let stream = storage.read_checked(keys, corrupt);
        return Ok((headers, Body::from_stream(stream)).into_response());
    }

    set(header::ETAG, etag.clone());
    set(header::CACHE_CONTROL, cache_control(fixed).into());
    set(header::ACCEPT_RANGES, "bytes".into());
    let header_is = |name: header::HeaderName| {
        request.get(name).and_then(|v| v.to_str().ok()).map(|v| {
            v.split(',')
                .any(|tag| tag.trim() == etag || tag.trim() == "*")
        })
    };
    if header_is(header::IF_NONE_MATCH) == Some(true) {
        return Ok((StatusCode::NOT_MODIFIED, headers).into_response());
    }
    let size = file.size as u64;
    // A range of an older version (`If-Range` naming another) is ignored: the whole file then.
    let range = match header_is(header::IF_RANGE) {
        Some(false) => None,
        _ => request
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| parse_range(v, size)),
    };
    match range {
        None => {
            set(header::CONTENT_LENGTH, size.to_string());
            let content = original(storage, &found.prefix, &file, 0, compression, corrupt);
            Ok((headers, Body::from_stream(content)).into_response())
        }
        Some(None) => {
            set(header::CONTENT_RANGE, format!("bytes */{size}"));
            Ok((StatusCode::RANGE_NOT_SATISFIABLE, headers).into_response())
        }
        Some(Some((start, end))) => {
            // From the part holding `start`: each part is read (and decompressed) whole.
            let first = start / PART_SIZE;
            let content = original(
                storage,
                &found.prefix,
                &file,
                first as i32,
                compression,
                corrupt,
            );
            let body = slice(content, start - first * PART_SIZE, end - start + 1);
            set(header::CONTENT_LENGTH, (end - start + 1).to_string());
            set(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"));
            Ok((
                StatusCode::PARTIAL_CONTENT,
                headers,
                Body::from_stream(body),
            )
                .into_response())
        }
    }
}

/// A `Range` header for a file of `size` bytes: `None` to ignore it (not one byte range we
/// read), `Some(None)` if it can't be satisfied, else the first and last byte.
fn parse_range(header: &str, size: u64) -> Option<Option<(u64, u64)>> {
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = (start.trim(), end.trim());
    let range = if start.is_empty() {
        // The last `n` bytes.
        let n: u64 = end.parse().ok()?;
        (n > 0 && size > 0).then(|| (size.saturating_sub(n), size - 1))
    } else {
        let start: u64 = start.parse().ok()?;
        let end = if end.is_empty() {
            u64::MAX
        } else {
            end.parse().ok()?
        };
        (start <= end && start < size).then(|| (start, end.min(size - 1)))
    };
    Some(range)
}

/// `take` bytes of `stream` after the first `skip`.
fn slice(
    stream: BoxStream<'static, std::io::Result<Bytes>>,
    skip: u64,
    take: u64,
) -> BoxStream<'static, std::io::Result<Bytes>> {
    stream
        .scan((skip, take), |(skip, take), item| {
            let out = match item {
                Err(err) => Some(Some(Err(err))),
                Ok(_) if *take == 0 => None,
                Ok(mut chunk) => {
                    let len = chunk.len() as u64;
                    if *skip >= len {
                        *skip -= len;
                        Some(None)
                    } else {
                        chunk = chunk.slice(*skip as usize..);
                        *skip = 0;
                        let keep = (chunk.len() as u64).min(*take);
                        chunk.truncate(keep as usize);
                        *take -= keep;
                        Some(Some(Ok(chunk)))
                    }
                }
            };
            futures_util::future::ready(out)
        })
        .filter_map(futures_util::future::ready)
        .boxed()
}

/// A file's content as it was uploaded, from part `first` on: its parts one after the other,
/// decompressed.
fn original(
    storage: &Storage,
    prefix: &str,
    file: &StoredFile,
    first: i32,
    compression: Compression,
    corrupt: Corrupt,
) -> BoxStream<'static, std::io::Result<Bytes>> {
    let stream = storage.read_checked(file.checked_keys(prefix, first), corrupt);
    match compression {
        Compression::None => stream,
        Compression::Zstd => {
            // Each part is a zstd frame: together, one zstd stream.
            let reader = tokio_util::io::StreamReader::new(stream);
            let mut decoder = async_compression::tokio::bufread::ZstdDecoder::new(reader);
            decoder.multiple_members(true);
            tokio_util::io::ReaderStream::new(decoder).boxed()
        }
    }
}

/// Told when storage gives back a part of a checkpoint that isn't what was uploaded.
type Corrupt = std::sync::Arc<dyn Fn(&object_store::path::Path) + Send + Sync>;

/// Marks the checkpoint corrupt (once): its page says so, and what.
fn report_corrupt(state: &AppState, id: Uuid) -> Corrupt {
    // The database only: a download in flight holds nothing else of the server's.
    let pg = state.pg().clone();
    std::sync::Arc::new(move |key: &object_store::path::Path| {
        let pg = pg.clone();
        let what = format!("storage gave back {key} changed: it isn't what was uploaded");
        tokio::spawn(async move {
            let marked = sqlx::query(
                "UPDATE checkpoints SET corrupt = $2 WHERE id = $1 AND corrupt IS NULL",
            )
            .bind(id)
            .bind(&what)
            .execute(&pg)
            .await;
            if let Err(err) = marked {
                tracing::warn!(checkpoint = %id, %err, "couldn't mark a checkpoint corrupt");
            }
        });
    })
}

/// `GET /api/v0/checkpoints/{id}/archive`, and its short link `GET /ckpt/{id}`: all of its files
/// in one tar archive, as they were uploaded, streamed as a download (`<run>-step-<step>.tar`).
pub async fn archive(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Response, UiError> {
    serve_archive(&state, &user, id, true).await
}

/// A checkpoint as one tar archive; `fixed` as for [`serve_path`].
pub(crate) async fn serve_archive(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
    fixed: bool,
) -> Result<Response, UiError> {
    let storage = storage(state)?;
    let found = committed(state, user, id).await?;
    let (run_name, step, mtime, digest): (String, i64, i64, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT r.name, c.step, extract(epoch from COALESCE(c.committed_at, c.created_at))::bigint,
                c.digest
         FROM checkpoints c JOIN runs r ON r.id = c.run_id WHERE c.id = $1",
    )
    .bind(id)
    .fetch_one(state.pg())
    .await?;
    let files: Vec<StoredFile> = sqlx::query_as(&format!(
        "SELECT {FILE_COLUMNS} FROM checkpoint_files WHERE checkpoint_id = $1 ORDER BY file_index"
    ))
    .bind(id)
    .fetch_all(state.pg())
    .await?;

    let length = tar::archive_len(files.iter().map(|f| (f.path.as_str(), f.size as u64)));
    let mut pieces: Vec<BoxStream<'static, std::io::Result<Bytes>>> = Vec::new();
    let bytes = |b: Vec<u8>| stream::once(async move { Ok(Bytes::from(b)) }).boxed();
    for (i, file) in files.iter().enumerate() {
        let size = file.size as u64;
        pieces.push(bytes(tar::file_header(
            i,
            &file.path,
            size,
            mtime.max(0) as u64,
        )));
        let compression = Compression::parse(&file.compression);
        pieces.push(original(
            storage,
            &found.prefix,
            file,
            0,
            compression,
            report_corrupt(state, found.id),
        ));
        pieces.push(bytes(vec![0; tar::padding(size)]));
    }
    pieces.push(bytes(tar::END.to_vec()));

    let name = download_name(&format!("{run_name}-step-{step}"));
    let mut response = (
        [
            (header::CONTENT_TYPE, "application/x-tar".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}.tar\""),
            ),
            (header::CONTENT_LENGTH, length.to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
            (header::CACHE_CONTROL, cache_control(fixed).to_owned()),
        ],
        Body::from_stream(stream::iter(pieces).flatten()),
    )
        .into_response();
    // The same checkpoint always gives the same archive: its digest names it.
    if let Some(digest) = digest {
        let digest = hex(&digest);
        let headers = response.headers_mut();
        headers.insert(header::ETAG, format!("\"{digest}\"").parse().expect("hex"));
        headers.insert("x-losscape-digest", digest.parse().expect("hex"));
    }
    Ok(response)
}

/// Content at a checkpoint's id never changes; at a name, `:latest` or a step, it can.
fn cache_control(fixed: bool) -> &'static str {
    if fixed {
        "private, max-age=31536000, immutable"
    } else {
        "private, no-cache"
    }
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

/// Every minute, or sooner when asked: pending checkpoints left too long go, so do those past
/// their project's retention, and the files of deleted ones are deleted from storage.
pub fn spawn_cleaner(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(err) = expire_pending(&state, PENDING_HOURS).await {
                tracing::warn!(%err.message, "expiring pending checkpoints failed");
            }
            if let Err(err) = retention::apply(&state).await {
                tracing::warn!(%err.message, "applying checkpoint retention failed");
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
            blake3: "ab".repeat(32),
            compression: Compression::None,
            parts: parts_of(size) as u32,
        }
    }

    #[test]
    fn ranges_are_read_as_http_means_them() {
        let r = |h: &str| parse_range(h, 100);
        assert_eq!(r("bytes=0-9"), Some(Some((0, 9))));
        assert_eq!(r("bytes=90-"), Some(Some((90, 99))));
        assert_eq!(
            r("bytes=90-500"),
            Some(Some((90, 99))),
            "clamped to the end"
        );
        assert_eq!(r("bytes=-10"), Some(Some((90, 99))), "the last 10");
        assert_eq!(r("bytes=-500"), Some(Some((0, 99))));
        assert_eq!(r("bytes=100-"), Some(None), "past the end");
        assert_eq!(r("bytes=5-2"), Some(None));
        assert_eq!(r("bytes=-0"), Some(None));
        assert_eq!(
            parse_range("bytes=0-", 0),
            Some(None),
            "nothing in an empty file"
        );
        for ignored in ["bytes=0-1,5-6", "items=0-1", "bytes=a-b", "bytes=1"] {
            assert_eq!(r(ignored), None, "{ignored}");
        }
    }

    #[tokio::test]
    async fn slices_cross_chunks() {
        let chunks: Vec<std::io::Result<Bytes>> = ["abc", "defg", "hij"]
            .iter()
            .map(|c| Ok(Bytes::from_static(c.as_bytes())))
            .collect();
        let out: Vec<Bytes> = slice(stream::iter(chunks).boxed(), 2, 6)
            .map(|c| c.unwrap())
            .collect()
            .await;
        assert_eq!(out.concat(), b"cdefgh");
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
        bad_hash.blake3 = "xyz".into();
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
