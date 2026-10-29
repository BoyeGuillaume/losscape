//! Compiles `proto/` into Rust types with protox (pure Rust), so contributors don't need `protoc`.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../proto");
    let files = [proto_root.join("losscape/v0/ingest.proto")];

    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
    }

    let descriptors = protox::compile(&files, [&proto_root])?;
    prost_build::Config::new().compile_fds(descriptors)?;
    Ok(())
}
