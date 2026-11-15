//! Code shared by the losscape server and the Python SDK core: protocol types and IDs.

pub mod ids;
pub mod proto;

/// Version of the SDK <-> server protocol, sent in the `X-Losscape-Protocol` header.
pub const PROTOCOL_VERSION: u32 = 1;

/// Header carrying [`PROTOCOL_VERSION`].
pub const PROTOCOL_HEADER: &str = "x-losscape-protocol";

/// Checkpoint files are uploaded in parts of this many bytes, before compression; the last
/// part of a file is shorter (docs/ARTIFACTS.md).
pub const CHECKPOINT_PART_SIZE: u64 = 16 * 1024 * 1024;

/// Header carrying the BLAKE3 hash of a checkpoint part as sent, or of a file as downloaded
/// (hex).
pub const PART_HASH_HEADER: &str = "x-losscape-blake3";

/// Version of the losscape crates.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
