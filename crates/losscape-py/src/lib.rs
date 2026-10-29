//! Native core of the losscape Python SDK, exposed as `losscape._native`.

use losscape_core::ids::RunId;
use pyo3::prelude::*;

/// Generate a new run ID (UUIDv7, 32 lowercase hex chars).
#[pyfunction]
fn new_run_id() -> String {
    RunId::new().to_string()
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", losscape_core::VERSION)?;
    m.add("PROTOCOL_VERSION", losscape_core::PROTOCOL_VERSION)?;
    m.add_function(wrap_pyfunction!(new_run_id, m)?)?;
    Ok(())
}
