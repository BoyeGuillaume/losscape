//! Client core shared by the losscape SDKs (Python via `losscape-py`).
//!
//! `log()` appends to an in-memory buffer; a background thread writes it as one batch
//! every ~100 ms to a local log on disk and uploads from there. Training never waits on
//! the network, and nothing is lost if the server is down or the process is killed:
//! `sync` uploads whatever is left.

pub mod buffer;
mod env;
mod run;
mod sync;
mod upload;
pub mod wal;

pub use run::{FinishOutcome, Mode, Run, RunOptions, StartError, WAKE_ROWS, now_ms};
pub use sync::{SyncResult, SyncStatus, sync};
pub use upload::UploadError;
