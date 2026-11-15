//! Checkpoints of a run (docs/ARTIFACTS.md): folders copied (or moved) into the run's local
//! folder when logged, then uploaded in the background like its metrics: file by file, in
//! parts, compressed when it's worth it. What isn't uploaded stays, for `losscape sync`.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{StreamExt, stream};
use losscape_core::CHECKPOINT_PART_SIZE as PART_SIZE;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinSet;

use crate::upload::{Client, Live, UploadError};
use crate::wal;

/// Inside a run's folder: one folder per step.
pub const DIR: &str = "checkpoints";
/// A checkpoint's files, as logged.
const FILES: &str = "files";
/// What's sent to create it, decided once so that resuming sends the same.
const MANIFEST: &str = "manifest.json";
/// Why the server refused it: it stays, and isn't sent again.
const REFUSED: &str = "refused.txt";
/// Parts sent at once, all checkpoints together (each in memory while sent).
const IN_FLIGHT: usize = 4;
/// Checkpoints uploaded at once: small ones are mostly round trips, which overlap.
const CHECKPOINTS_AT_ONCE: usize = 8;
const ZSTD_LEVEL: i32 = 1;
/// Compression is tried on this much of a file...
const SAMPLE_BYTES: usize = 4 << 20;
/// ...and used if it saves at least this much of it.
const MIN_SAVING: f64 = 0.05;

/// Bytes of checkpoints uploaded, and to upload: for progress.
#[derive(Default)]
pub struct Progress {
    pub sent: AtomicU64,
    pub total: AtomicU64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Manifest {
    step: u64,
    files: Vec<FileEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct FileEntry {
    path: String,
    size: u64,
    /// BLAKE3 of its content, which the server computes again from the parts it receives.
    blake3: String,
    compression: String,
    parts: u64,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// Copies `src` (a folder, or one file) into the run's folder as its checkpoint at `step`, or
/// with `take` moves it there (a rename on the same disk; a copy, then deleting the original,
/// across disks). Returns where its files are, until uploaded. The checkpoint only appears,
/// to be uploaded, once complete.
pub fn snapshot(run_dir: &Path, src: &Path, step: u64, take: bool) -> io::Result<PathBuf> {
    let root = run_dir.join(DIR);
    let target = root.join(step.to_string());
    if target.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("a checkpoint at step {step} is already waiting to upload"),
        ));
    }
    let meta = fs::metadata(src)?;
    if meta.is_dir() && count_files(src)? == 0 {
        return Err(invalid(format!("{}: no files in it", src.display())));
    }
    wal::create_private_dir(&root)?;
    let partial = root.join(format!(".{step}.partial"));
    if partial.exists() {
        fs::remove_dir_all(&partial)?;
    }
    fs::create_dir(&partial)?;
    let files = partial.join(FILES);
    let copied = if meta.is_dir() {
        if take && fs::rename(src, &files).is_ok() {
            Ok(())
        } else {
            copy_tree(src, &files)
        }
    } else {
        let name = src
            .file_name()
            .ok_or_else(|| invalid(format!("{}: not a file or folder", src.display())))?;
        fs::create_dir(&files).and_then(|()| {
            let dst = files.join(name);
            if take && fs::rename(src, &dst).is_ok() {
                Ok(())
            } else {
                fs::copy(src, &dst).map(|_| ())
            }
        })
    };
    if let Err(err) = copied {
        fs::remove_dir_all(&partial).ok();
        return Err(err);
    }
    fs::rename(&partial, &target)?;
    // Moved across disks: the original goes only now, with the copy complete.
    if take && src.exists() {
        if meta.is_dir() {
            fs::remove_dir_all(src)?;
        } else {
            fs::remove_file(src)?;
        }
    }
    Ok(target.join(FILES))
}

/// Copies files and folders, following links to files. Links to folders are left out: they
/// could loop, or reach far outside the checkpoint.
fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let link = entry.file_type()?.is_symlink();
        let meta = fs::metadata(&from)?;
        if meta.is_dir() && !link {
            copy_tree(&from, &to)?;
        } else if meta.is_file() {
            // A reflink where the disk can (btrfs, XFS): instant, and no space used.
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn count_files(dir: &Path) -> io::Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let link = entry.file_type()?.is_symlink();
        let meta = fs::metadata(entry.path())?;
        if meta.is_dir() && !link {
            count += count_files(&entry.path())?;
        } else if meta.is_file() {
            count += 1;
        }
    }
    Ok(count)
}

/// Checkpoints of a run folder waiting to upload, by step; refused ones aren't.
pub fn ready(run_dir: &Path) -> io::Result<Vec<(u64, PathBuf)>> {
    Ok(local(run_dir)?
        .into_iter()
        .filter(|(_, dir)| !dir.join(REFUSED).exists())
        .collect())
}

/// Checkpoints the server refused, with why.
pub fn refused(run_dir: &Path) -> io::Result<Vec<(u64, String)>> {
    let mut out = Vec::new();
    for (step, dir) in local(run_dir)? {
        if let Ok(reason) = fs::read_to_string(dir.join(REFUSED)) {
            out.push((step, reason));
        }
    }
    Ok(out)
}

/// Every checkpoint still in a run folder (waiting, or refused), by step, with where its files
/// are.
pub fn local_files(run_dir: &Path) -> io::Result<Vec<(u64, PathBuf)>> {
    Ok(local(run_dir)?
        .into_iter()
        .map(|(step, dir)| (step, dir.join(FILES)))
        .collect())
}

fn local(run_dir: &Path) -> io::Result<Vec<(u64, PathBuf)>> {
    let entries = match fs::read_dir(run_dir.join(DIR)) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry?;
        // `.<step>.partial` folders are copies that never finished: not checkpoints.
        let Some(step) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u64>().ok())
        else {
            continue;
        };
        if entry.path().join(FILES).is_dir() {
            out.push((step, entry.path()));
        }
    }
    out.sort();
    Ok(out)
}

/// Removes copies left unfinished by a process that stopped while making them.
pub fn remove_partials(run_dir: &Path) {
    if let Ok(entries) = fs::read_dir(run_dir.join(DIR)) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().ends_with(".partial") {
                fs::remove_dir_all(entry.path()).ok();
            }
        }
    }
}

/// The checkpoint's files, sizes, hashes and compression: read from what was decided before,
/// or decided now (hashing every file).
fn manifest(dir: &Path, step: u64) -> io::Result<Manifest> {
    if let Ok(bytes) = fs::read(dir.join(MANIFEST))
        && let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes)
    {
        return Ok(manifest);
    }
    let files_dir = dir.join(FILES);
    let mut paths = Vec::new();
    list(&files_dir, &files_dir, &mut paths)?;
    paths.sort();
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let full = files_dir.join(&path);
        let size = fs::metadata(&full)?.len();
        files.push(FileEntry {
            blake3: hash_file(&full)?,
            compression: if compresses(&full)? { "zstd" } else { "none" }.into(),
            parts: size.div_ceil(PART_SIZE),
            size,
            path,
        });
    }
    let manifest = Manifest { step, files };
    wal::write_atomic(
        &dir.join(MANIFEST),
        &serde_json::to_vec(&manifest).map_err(io::Error::other)?,
    )?;
    Ok(manifest)
}

/// Paths of the files under `dir`, relative to `root`, with `/` between names.
fn list(root: &Path, dir: &Path, out: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            list(root, &path, out)?;
            continue;
        }
        let relative = path.strip_prefix(root).expect("under root");
        let names: Option<Vec<&str>> = relative.iter().map(|part| part.to_str()).collect();
        let names = names
            .ok_or_else(|| invalid(format!("{}: file names must be UTF-8", relative.display())))?;
        out.push(names.join("/"));
    }
    Ok(())
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Whether zstd saves enough on the start of the file: weights barely compress, configs and
/// mostly-zero tensors do.
fn compresses(path: &Path) -> io::Result<bool> {
    let mut sample = Vec::with_capacity(SAMPLE_BYTES);
    fs::File::open(path)?
        .take(SAMPLE_BYTES as u64)
        .read_to_end(&mut sample)?;
    if sample.is_empty() {
        return Ok(false);
    }
    let compressed = zstd::bulk::compress(&sample, ZSTD_LEVEL)?;
    Ok((compressed.len() as f64) <= sample.len() as f64 * (1.0 - MIN_SAVING))
}

/// A part of a file as it's sent: compressed (one zstd frame) or not, with its hash and how
/// many bytes of the file it holds.
fn read_part(path: &Path, part: u64, zstd: bool) -> io::Result<(Bytes, String, u64)> {
    let mut file = fs::File::open(path)?;
    file.seek(SeekFrom::Start(part * PART_SIZE))?;
    let mut data = Vec::with_capacity(PART_SIZE as usize);
    file.take(PART_SIZE).read_to_end(&mut data)?;
    let original = data.len() as u64;
    if zstd {
        data = zstd::bulk::compress(&data, ZSTD_LEVEL)?;
    }
    let hash = blake3::hash(&data).to_hex().to_string();
    Ok((data.into(), hash, original))
}

/// Uploads one checkpoint, then deletes it locally. Sent again, the same manifest picks up
/// where the last attempt stopped: only missing parts are sent.
async fn upload(
    client: &Client,
    dir: &Path,
    step: u64,
    run_id: &str,
    progress: &Progress,
    parts: &Arc<Semaphore>,
) -> Result<(), UploadError> {
    let manifest = {
        let dir = dir.to_owned();
        tokio::task::spawn_blocking(move || manifest(&dir, step))
            .await
            .expect("hashing doesn't panic")?
    };
    let body = serde_json::to_value(&manifest).expect("serializable");
    let created = client
        .json(
            Method::POST,
            &format!("runs/{run_id}/checkpoints"),
            Some(&body),
        )
        .await?;
    let id = created["id"]
        .as_str()
        .ok_or_else(|| UploadError::Local("checkpoint created without an id".into()))?
        .to_owned();

    let total: u64 = manifest.files.iter().map(|f| f.size).sum();
    progress.total.fetch_add(total, Ordering::SeqCst);
    let mut todo = Vec::new();
    for (index, file) in manifest.files.iter().enumerate() {
        let have: HashSet<u64> = created["files"][index]["uploaded"]
            .as_array()
            .map(|parts| parts.iter().filter_map(|p| p.as_u64()).collect())
            .unwrap_or_default();
        for part in 0..file.parts {
            if have.contains(&part) {
                let length = (file.size - part * PART_SIZE).min(PART_SIZE);
                progress.sent.fetch_add(length, Ordering::SeqCst);
            } else {
                todo.push((index, part));
            }
        }
    }

    let mut todo = todo.into_iter();
    let mut sending = JoinSet::new();
    loop {
        while sending.len() < IN_FLIGHT {
            let Some((index, part)) = todo.next() else {
                break;
            };
            let file = &manifest.files[index];
            let path = dir.join(FILES).join(&file.path);
            let zstd = file.compression == "zstd";
            let client = client.clone();
            let url = format!("checkpoints/{id}/files/{index}/parts/{part}");
            let parts = parts.clone();
            sending.spawn(async move {
                let _permit = parts.acquire_owned().await.expect("never closed");
                let (body, hash, length) =
                    tokio::task::spawn_blocking(move || read_part(&path, part, zstd))
                        .await
                        .expect("reading doesn't panic")?;
                client.put_part(&url, body, &hash).await.map(|()| length)
            });
        }
        let Some(done) = sending.join_next().await else {
            break;
        };
        let length = done.expect("upload task doesn't panic")?;
        progress.sent.fetch_add(length, Ordering::SeqCst);
    }

    client
        .json(Method::POST, &format!("checkpoints/{id}/commit"), None)
        .await?;
    fs::remove_dir_all(dir)?;
    Ok(())
}

/// Uploads the run's checkpoints waiting to upload, by step. One the server refuses is kept
/// with why, and not sent again; other errors stop here (to be tried again).
pub async fn upload_ready(
    client: &Client,
    run_dir: &Path,
    run_id: &str,
    progress: &Progress,
) -> Result<(), UploadError> {
    let parts = Arc::new(Semaphore::new(IN_FLIGHT));
    let mut uploads = stream::iter(ready(run_dir)?)
        .map(|(step, dir)| {
            let parts = &parts;
            async move {
                let result = upload(client, &dir, step, run_id, progress, parts).await;
                (step, dir, result)
            }
        })
        .buffer_unordered(CHECKPOINTS_AT_ONCE);
    while let Some((step, dir, result)) = uploads.next().await {
        match result {
            Ok(()) => {}
            // Not this checkpoint's fault: kept to try again (with `losscape sync`).
            Err(err @ UploadError::Rejected { .. }) if err.code() == Some("no_storage") => {
                return Err(err);
            }
            Err(UploadError::Rejected { code, message, .. }) => {
                let reason = if message.is_empty() { code } else { message };
                eprintln!(
                    "losscape: checkpoint at step {step} not uploaded: {reason}. It stays in {}",
                    dir.display()
                );
                fs::write(dir.join(REFUSED), &reason)?;
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// A live run's checkpoints: uploads each one as it's logged, once the run exists on the
/// server, until the run finished and none is left. Returns why some stay locally, if any:
/// it never fails, so that metrics keep uploading whatever happens here.
pub async fn upload_live(
    client: &Client,
    run_dir: &Path,
    run_id: &str,
    live: &Live,
    wake: &Notify,
    progress: &Progress,
) -> Result<Option<String>, UploadError> {
    while !live.initialized.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    loop {
        // Read first: checkpoints logged before the run finished are all there by then.
        let finished = live.finished.load(Ordering::SeqCst);
        if let Err(err) = upload_ready(client, run_dir, run_id, progress).await {
            eprintln!(
                "losscape: checkpoint upload stopped: {err}. They stay in {}; upload them later with `losscape sync`.",
                run_dir.join(DIR).display()
            );
            return Ok(Some(format!("checkpoints not uploaded: {err}")));
        }
        if finished {
            return Ok(refused(run_dir)?.first().map(|(step, reason)| {
                format!("checkpoint at step {step} refused by the server: {reason}")
            }));
        }
        tokio::time::timeout(Duration::from_secs(1), wake.notified())
            .await
            .ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn snapshots_copy_or_move_whole_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let run = tmp.path().join("run");
        let src = tmp.path().join("ckpt");
        write(&src.join("model.pt"), b"weights");
        write(&src.join("tok/vocab.json"), b"{}");

        let files = snapshot(&run, &src, 100, false).unwrap();
        assert_eq!(fs::read(files.join("tok/vocab.json")).unwrap(), b"{}");
        assert!(src.exists(), "copied: the original stays");
        let err = snapshot(&run, &src, 100, false).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "one per step");

        // The script saving again to the same place doesn't change what was logged.
        write(&src.join("model.pt"), b"newer");
        assert_eq!(fs::read(files.join("model.pt")).unwrap(), b"weights");

        let files = snapshot(&run, &src, 200, true).unwrap();
        assert!(!src.exists(), "moved");
        assert_eq!(fs::read(files.join("model.pt")).unwrap(), b"newer");
        let steps: Vec<u64> = ready(&run).unwrap().into_iter().map(|(s, _)| s).collect();
        assert_eq!(steps, [100, 200]);
    }

    #[test]
    fn snapshots_of_one_file_and_refusals() {
        let tmp = tempfile::tempdir().unwrap();
        let run = tmp.path().join("run");
        write(&tmp.path().join("last.pt"), b"w");
        let files = snapshot(&run, &tmp.path().join("last.pt"), 0, false).unwrap();
        assert_eq!(fs::read(files.join("last.pt")).unwrap(), b"w");

        fs::create_dir(tmp.path().join("empty")).unwrap();
        assert!(snapshot(&run, &tmp.path().join("empty"), 1, false).is_err());
        assert!(snapshot(&run, &tmp.path().join("missing"), 1, false).is_err());
        assert!(
            !run.join(DIR).join("1").exists() && !run.join(DIR).join(".1.partial").exists(),
            "nothing left behind"
        );
    }

    #[cfg(unix)]
    #[test]
    fn links_to_files_are_followed_and_links_to_folders_left_out() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("ckpt");
        write(&tmp.path().join("blob"), b"real");
        write(&src.join("a"), b"a");
        std::os::unix::fs::symlink(tmp.path().join("blob"), src.join("linked")).unwrap();
        std::os::unix::fs::symlink(tmp.path(), src.join("loop")).unwrap();
        let files = snapshot(&tmp.path().join("run"), &src, 0, false).unwrap();
        assert_eq!(fs::read(files.join("linked")).unwrap(), b"real");
        assert!(!files.join("loop").exists());
    }

    #[test]
    fn manifests_decide_compression_once() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("ckpt");
        let noise: Vec<u8> = (0..3_000u64)
            .flat_map(|i| *blake3::hash(&i.to_le_bytes()).as_bytes())
            .collect();
        write(&src.join("weights.bin"), &noise);
        write(&src.join("config.json"), &br#"{"a": 1}"#.repeat(1000));
        write(&src.join("empty"), b"");
        let files = snapshot(&tmp.path().join("run"), &src, 7, false).unwrap();
        let dir = files.parent().unwrap();

        let m = manifest(dir, 7).unwrap();
        let by_path = |p: &str| m.files.iter().find(|f| f.path == p).unwrap().clone();
        assert_eq!(by_path("weights.bin").compression, "none");
        assert_eq!(by_path("config.json").compression, "zstd");
        assert_eq!(by_path("empty").parts, 0);
        assert_eq!(
            by_path("weights.bin").blake3,
            hex(blake3::hash(&noise).as_bytes())
        );
        // Read back as decided, even if the files were to change.
        fs::write(files.join("config.json"), &noise).unwrap();
        assert_eq!(manifest(dir, 7).unwrap(), m);
    }

    #[test]
    fn parts_cover_the_file_and_decompress_to_it() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("f");
        let data: Vec<u8> = (0..PART_SIZE + 100).map(|i| (i % 7) as u8).collect();
        fs::write(&path, &data).unwrap();
        let (first, hash, len) = read_part(&path, 0, false).unwrap();
        assert_eq!((first.len() as u64, len), (PART_SIZE, PART_SIZE));
        assert_eq!(hash, hex(blake3::hash(&first).as_bytes()));
        let (second, _, len) = read_part(&path, 1, true).unwrap();
        assert_eq!(len, 100);
        assert_eq!(
            zstd::decode_all(second.as_ref()).unwrap(),
            data[PART_SIZE as usize..]
        );
    }
}
