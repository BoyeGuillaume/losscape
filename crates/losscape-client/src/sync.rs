//! `losscape sync`: uploads runs left in the local log (offline mode, a server that was
//! unreachable, or a process that was killed).

use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

use crate::upload::{Client, UploadError, upload_run};
use crate::wal;

/// Give up on a run if the server stays unreachable this long.
const RETRY_FOR: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq)]
pub enum SyncStatus {
    Uploaded,
    /// Its process is still running and uploads it itself.
    InUse,
    Failed(String),
}

#[derive(Debug)]
pub struct SyncResult {
    pub run_id: String,
    pub status: SyncStatus,
}

pub fn sync(root: &Path, server: &str, api_key: &str) -> io::Result<Vec<SyncResult>> {
    let runs = root.join("runs");
    let mut dirs: Vec<_> = match fs::read_dir(&runs) {
        Ok(entries) => entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    dirs.sort();

    let client = Client::new(server, api_key, Some(RETRY_FOR)).map_err(io::Error::other)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut results = Vec::new();
    for dir in dirs {
        let run_id = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let status = match wal::try_lock(&dir)? {
            None => SyncStatus::InUse,
            Some(lock) => {
                let result = runtime.block_on(upload_run(&client, &dir, None));
                drop(lock);
                match result {
                    Ok(()) => {
                        fs::remove_dir_all(&dir)?;
                        SyncStatus::Uploaded
                    }
                    Err(UploadError::Unreachable(reason)) => {
                        SyncStatus::Failed(format!("server unreachable: {reason}"))
                    }
                    Err(err) => SyncStatus::Failed(err.to_string()),
                }
            }
        };
        results.push(SyncResult { run_id, status });
    }
    Ok(results)
}
