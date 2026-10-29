//! Code shared by the losscape server and the Python SDK core: protocol types and IDs.

pub mod ids;
pub mod proto;

/// Version of the SDK <-> server protocol, sent in the `X-Losscape-Protocol` header.
pub const PROTOCOL_VERSION: u32 = 0;

/// Header carrying [`PROTOCOL_VERSION`].
pub const PROTOCOL_HEADER: &str = "x-losscape-protocol";

/// Version of the losscape crates.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
