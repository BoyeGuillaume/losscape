//! Local write-ahead log of a run: `<dir>/runs/<run id>/`.
//!
//! - `init.pb`: the run's `RunInit`, so `losscape sync` can create it on the server.
//! - `seg-<first seq>.wal`: batches, appended as `seq u64 | len u32 | crc32 u32 | payload`
//!   (payload = zstd-compressed `LogBatch`). Rotated every few MB.
//! - `acked`: every seq below this number is confirmed by the server. Segments entirely
//!   below it are deleted.
//! - `finish.pb`: written when the run ends normally.
//! - `lock`: held (OS file lock) by the process that owns the run.
//!
//! Batches reach the page cache as soon as they are written, so they survive the
//! process being killed. The directory is deleted once everything is uploaded.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"DFWAL001";
const HEADER: usize = 16;
const SEGMENT_MAX_BYTES: u64 = 8 * 1024 * 1024;

pub const INIT_FILE: &str = "init.pb";
pub const FINISH_FILE: &str = "finish.pb";
const ACKED_FILE: &str = "acked";
const LOCK_FILE: &str = "lock";

/// Holds the run's lock file for as long as it lives.
pub struct RunLock(#[allow(dead_code)] File);

/// `Ok(None)` if another live process owns the run.
pub fn try_lock(dir: &Path) -> io::Result<Option<RunLock>> {
    let file = private(OpenOptions::new().create(true).truncate(false).write(true))
        .open(dir.join(LOCK_FILE))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(RunLock(file))),
        Err(fs::TryLockError::WouldBlock) => Ok(None),
        Err(fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Writes `path` atomically (temp file + rename).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    // Never through something already there: a link left by someone else, on a shared disk.
    match fs::remove_file(&tmp) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
        _ => {}
    }
    private(OpenOptions::new().write(true).create_new(true))
        .open(&tmp)?
        .write_all(bytes)?;
    fs::rename(tmp, path)
}

/// The local log holds everything a run records, on disks that are often shared: for its
/// owner only. On Windows, files inherit the permissions of the folder.
#[cfg(unix)]
fn private(options: &mut OpenOptions) -> &mut OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600)
}

#[cfg(not(unix))]
fn private(options: &mut OpenOptions) -> &mut OpenOptions {
    options
}

/// Creates a run's folder, and those above it if missing, for its owner only. If folders above
/// were there already, they stay as they are.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(0o700);
        builder.create(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    builder.create(dir)
}

pub fn read_acked(dir: &Path) -> io::Result<u64> {
    match fs::read(dir.join(ACKED_FILE)) {
        Ok(bytes) => Ok(bytes.try_into().map(u64::from_le_bytes).unwrap_or(0)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e),
    }
}

/// Records that every seq below `acked` is confirmed, and deletes segments that only
/// hold such seqs.
pub fn write_acked(dir: &Path, acked: u64) -> io::Result<()> {
    write_atomic(&dir.join(ACKED_FILE), &acked.to_le_bytes())?;
    let segments = list_segments(dir)?;
    for pair in segments.windows(2) {
        // A segment ends where the next begins.
        if pair[1].0 <= acked {
            fs::remove_file(&pair[0].1)?;
        }
    }
    Ok(())
}

fn segment_path(dir: &Path, first_seq: u64) -> PathBuf {
    dir.join(format!("seg-{first_seq:020}.wal"))
}

/// Segments sorted by first seq.
fn list_segments(dir: &Path) -> io::Result<Vec<(u64, PathBuf)>> {
    let mut segments = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let first = path.file_name().and_then(|n| n.to_str()).and_then(|n| {
            n.strip_prefix("seg-")?
                .strip_suffix(".wal")?
                .parse::<u64>()
                .ok()
        });
        if let Some(first) = first {
            segments.push((first, path));
        }
    }
    segments.sort();
    Ok(segments)
}

/// Reads one record at the reader's position. `Ok(None)` at a clean end, a partially
/// written record, or a corrupt one (all mean "nothing more to read here for now").
fn read_record(file: &mut File) -> io::Result<Option<(u64, Vec<u8>)>> {
    let start = file.stream_position()?;
    let mut header = [0u8; HEADER];
    if !read_full(file, &mut header)? {
        file.seek(SeekFrom::Start(start))?;
        return Ok(None);
    }
    let seq = u64::from_le_bytes(header[0..8].try_into().unwrap());
    let len = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let crc = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let mut payload = vec![0u8; len];
    if !read_full(file, &mut payload)? || crc32fast::hash(&payload) != crc {
        file.seek(SeekFrom::Start(start))?;
        return Ok(None);
    }
    Ok(Some((seq, payload)))
}

/// Like `read_exact`, but `Ok(false)` instead of an error at end of file.
fn read_full(file: &mut File, buf: &mut [u8]) -> io::Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => return Ok(false),
            n => filled += n,
        }
    }
    Ok(true)
}

fn open_segment(path: &Path) -> io::Result<File> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 8];
    if !read_full(&mut file, &mut magic)? || &magic != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is not a WAL segment", path.display()),
        ));
    }
    Ok(file)
}

pub struct WalWriter {
    dir: PathBuf,
    file: Option<File>,
    segment_bytes: u64,
    next_seq: u64,
}

impl WalWriter {
    /// Opens a run directory, continuing after the last complete record. A record torn by
    /// a crash mid-write is cut off.
    pub fn open(dir: &Path) -> io::Result<Self> {
        let mut writer = Self {
            dir: dir.to_owned(),
            file: None,
            segment_bytes: 0,
            next_seq: read_acked(dir)?,
        };
        if let Some((_, path)) = list_segments(dir)?.pop() {
            let mut file = open_segment(&path)?;
            while let Some((seq, _)) = read_record(&mut file)? {
                writer.next_seq = writer.next_seq.max(seq + 1);
            }
            let end = file.stream_position()?;
            let file = OpenOptions::new().write(true).open(&path)?;
            file.set_len(end)?;
            let mut file = file;
            file.seek(SeekFrom::End(0))?;
            writer.segment_bytes = end;
            writer.file = Some(file);
        }
        Ok(writer)
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// For a resumed run with no local state: start numbering after what the server has.
    pub fn start_at(&mut self, seq: u64) -> io::Result<()> {
        if seq > self.next_seq {
            self.next_seq = seq;
            write_acked(&self.dir, seq)?;
        }
        Ok(())
    }

    /// Appends a batch with the next seq and returns that seq.
    pub fn append(&mut self, payload: &[u8]) -> io::Result<u64> {
        let seq = self.next_seq;
        if self.file.is_none() || self.segment_bytes >= SEGMENT_MAX_BYTES {
            let mut file = private(OpenOptions::new().create_new(true).write(true))
                .open(segment_path(&self.dir, seq))?;
            file.write_all(MAGIC)?;
            self.file = Some(file);
            self.segment_bytes = MAGIC.len() as u64;
        }
        let mut record = Vec::with_capacity(HEADER + payload.len());
        record.extend_from_slice(&seq.to_le_bytes());
        record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        record.extend_from_slice(&crc32fast::hash(payload).to_le_bytes());
        record.extend_from_slice(payload);
        // One write call, so a reader following the file sees whole records or nothing.
        self.file
            .as_mut()
            .expect("opened above")
            .write_all(&record)?;
        self.segment_bytes += record.len() as u64;
        self.next_seq += 1;
        Ok(seq)
    }
}

/// Reads batches in seq order, following the log while it is being written.
pub struct WalReader {
    dir: PathBuf,
    current: Option<(u64, File)>,
    from_seq: u64,
}

impl WalReader {
    pub fn new(dir: &Path, from_seq: u64) -> Self {
        Self {
            dir: dir.to_owned(),
            current: None,
            from_seq,
        }
    }

    /// The next batch at or after `from_seq`, or `None` if none is complete yet.
    pub fn next_batch(&mut self) -> io::Result<Option<(u64, Vec<u8>)>> {
        loop {
            if self.current.is_none() {
                let segments = list_segments(&self.dir)?;
                // The last segment starting at or before the position, else the first after it.
                let pick = segments
                    .iter()
                    .rev()
                    .find(|(first, _)| *first <= self.from_seq)
                    .or_else(|| segments.iter().find(|(first, _)| *first > self.from_seq));
                match pick {
                    Some((first, path)) => self.current = Some((*first, open_segment(path)?)),
                    None => return Ok(None),
                }
            }
            let (first, file) = self.current.as_mut().expect("set above");
            while let Some((seq, payload)) = read_record(file)? {
                if seq >= self.from_seq {
                    self.from_seq = seq + 1;
                    return Ok(Some((seq, payload)));
                }
            }
            // End of this segment: move on only if a later one exists (the writer is done
            // with this one), otherwise wait for more data here.
            let first = *first;
            match list_segments(&self.dir)?
                .into_iter()
                .find(|(f, _)| *f > first)
            {
                Some((next_first, _)) => {
                    self.from_seq = self.from_seq.max(next_first);
                    self.current = None;
                }
                None => return Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(dir: &Path, from: u64) -> Vec<(u64, Vec<u8>)> {
        let mut reader = WalReader::new(dir, from);
        std::iter::from_fn(|| reader.next_batch().unwrap()).collect()
    }

    #[test]
    fn append_then_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = WalWriter::open(dir.path()).unwrap();
        for i in 0..5u8 {
            assert_eq!(w.append(&[i; 10]).unwrap(), i as u64);
        }
        let records = read_all(dir.path(), 0);
        assert_eq!(records.len(), 5);
        assert_eq!(records[3], (3, vec![3; 10]));
        assert_eq!(read_all(dir.path(), 3).len(), 2);
    }

    #[test]
    fn reader_follows_appends() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = WalWriter::open(dir.path()).unwrap();
        let mut r = WalReader::new(dir.path(), 0);
        assert_eq!(r.next_batch().unwrap(), None);
        w.append(b"one").unwrap();
        assert_eq!(r.next_batch().unwrap(), Some((0, b"one".to_vec())));
        assert_eq!(r.next_batch().unwrap(), None);
        w.append(b"two").unwrap();
        assert_eq!(r.next_batch().unwrap(), Some((1, b"two".to_vec())));
    }

    #[test]
    fn rotates_segments_and_reads_across_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = WalWriter::open(dir.path()).unwrap();
        let big = vec![7u8; 3 * 1024 * 1024];
        let mut r = WalReader::new(dir.path(), 0);
        for _ in 0..6 {
            w.append(&big).unwrap();
        }
        assert!(list_segments(dir.path()).unwrap().len() >= 2);
        let seqs: Vec<u64> = std::iter::from_fn(|| r.next_batch().unwrap())
            .map(|(s, _)| s)
            .collect();
        assert_eq!(seqs, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn acked_deletes_finished_segments() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = WalWriter::open(dir.path()).unwrap();
        let big = vec![1u8; 5 * 1024 * 1024];
        for _ in 0..4 {
            w.append(&big).unwrap();
        }
        let before = list_segments(dir.path()).unwrap();
        assert_eq!(before.iter().map(|s| s.0).collect::<Vec<_>>(), [0, 2]);
        write_acked(dir.path(), 2).unwrap();
        assert_eq!(list_segments(dir.path()).unwrap().len(), 1);
        assert_eq!(read_acked(dir.path()).unwrap(), 2);
        assert_eq!(read_all(dir.path(), 2).len(), 2);
    }

    #[test]
    fn reopen_cuts_torn_record_and_continues() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = WalWriter::open(dir.path()).unwrap();
        w.append(b"good").unwrap();
        w.append(b"also good").unwrap();
        drop(w);
        // Simulate a crash in the middle of writing a third record.
        let (_, path) = list_segments(dir.path()).unwrap().pop().unwrap();
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(&[2, 0, 0, 0, 0, 0, 0, 0, 100, 0]).unwrap();
        drop(f);

        assert_eq!(
            read_all(dir.path(), 0).len(),
            2,
            "torn record is not returned"
        );
        let mut w = WalWriter::open(dir.path()).unwrap();
        assert_eq!(w.next_seq(), 2);
        w.append(b"after crash").unwrap();
        let records = read_all(dir.path(), 0);
        assert_eq!(records.last().unwrap(), &(2, b"after crash".to_vec()));
    }

    #[test]
    fn lock_is_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let held = try_lock(dir.path()).unwrap();
        assert!(held.is_some());
        assert!(try_lock(dir.path()).unwrap().is_none());
        drop(held);
        assert!(try_lock(dir.path()).unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn the_local_log_is_for_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        // Folders from an earlier version, and a umask that would leave everything readable.
        let dir = root.path().join("runs/one");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        create_private_dir(&dir).unwrap();
        let fresh = root.path().join("new/runs/two");
        create_private_dir(&fresh).unwrap();

        let mode = |path: &Path| path.metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&fresh), 0o700);
        assert_eq!(mode(&root.path().join("new")), 0o700);

        let _lock = try_lock(&dir).unwrap();
        let mut writer = WalWriter::open(&dir).unwrap();
        writer.append(b"batch").unwrap();
        write_atomic(&dir.join("init.pb"), b"init").unwrap();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            assert_eq!(mode(&path), 0o600, "{}", path.display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn writing_doesnt_follow_links_left_by_someone_else() {
        let root = tempfile::tempdir().unwrap();
        let victim = root.path().join("victim");
        std::fs::write(&victim, b"precious").unwrap();
        let target = root.path().join("acked");
        std::os::unix::fs::symlink(&victim, target.with_extension("tmp")).unwrap();
        write_atomic(&target, b"12345678").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
        assert_eq!(std::fs::read(&target).unwrap(), b"12345678");
    }
}
