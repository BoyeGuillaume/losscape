//! Native core of the losscape Python SDK, exposed as `losscape._native`.
//! The Python-facing API lives in `sdk/python/src/losscape`.

use std::path::PathBuf;
use std::time::Duration;

use losscape_client::buffer::Buffer;
use losscape_client::{
    EnvExtra, FinishOutcome, Mode, Run, RunOptions, StartError, SyncStatus, now_ms,
};
use losscape_core::ids::RunId;
use losscape_core::proto::v0::RunExitState;
use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyString};
use serde_json::{Map, Value};

/// Generate a new run ID (UUIDv7, 32 lowercase hex chars).
#[pyfunction]
fn new_run_id() -> String {
    RunId::new().to_string()
}

#[pyclass(frozen, module = "losscape._native")]
struct NativeRun {
    run: Run,
}

#[pymethods]
impl NativeRun {
    #[getter]
    fn id(&self) -> String {
        self.run.id().to_string()
    }

    #[getter]
    fn dir(&self) -> PathBuf {
        self.run.dir().to_owned()
    }

    /// For a resumed run: the first step after what was already logged.
    /// Someone asked for this run to stop (the UI's Abort).
    #[getter]
    fn abort_requested(&self) -> bool {
        self.run.abort_requested()
    }

    #[getter]
    fn next_step(&self) -> Option<u64> {
        self.run.next_step()
    }

    /// Appends numbers (nested dicts are flattened with "/") at `step`. All-or-nothing:
    /// on a bad value nothing from this call is kept.
    fn log(&self, data: &Bound<'_, PyDict>, step: u64) -> PyResult<()> {
        let now = now_ms();
        let rows = {
            let mut buffer = self.run.buffer();
            let before = buffer.rows();
            if let Err(err) = push_dict(&mut buffer, data, None, step, now) {
                buffer.truncate(before);
                return Err(err);
            }
            buffer.rows()
        };
        self.run.after_push(rows);
        Ok(())
    }

    /// Merges a JSON object into the run summary.
    fn update_summary(&self, json: &str) -> PyResult<()> {
        self.run.update_summary(parse_object(json)?);
        Ok(())
    }

    /// Merges a JSON object into the run config.
    fn update_config(&self, json: &str) -> PyResult<()> {
        self.run.update_config(parse_object(json)?);
        Ok(())
    }

    /// Merges a JSON object into the run's model descriptions, by model name.
    fn update_models(&self, json: &str) -> PyResult<()> {
        self.run.update_models(parse_object(json)?);
        Ok(())
    }

    /// Returns (outcome, message, pending batches); outcome is "uploaded",
    /// "saved_locally" or "failed". Releases the GIL while waiting.
    fn finish(
        &self,
        py: Python<'_>,
        state: &str,
        exit_code: i32,
        timeout: f64,
    ) -> PyResult<(String, String, u64)> {
        let state = match state {
            "finished" => RunExitState::Finished,
            "failed" => RunExitState::Failed,
            "killed" => RunExitState::Killed,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown run state {other:?}"
                )));
            }
        };
        let timeout = Duration::from_secs_f64(timeout.max(0.0));
        let outcome = py.detach(|| self.run.finish(state, exit_code, timeout));
        Ok(match outcome {
            FinishOutcome::Uploaded => ("uploaded".into(), String::new(), 0),
            FinishOutcome::SavedLocally {
                pending_batches,
                reason,
            } => ("saved_locally".into(), reason, pending_batches),
            FinishOutcome::Failed(reason) => ("failed".into(), reason, 0),
        })
    }
}

fn push_dict(
    buffer: &mut Buffer,
    data: &Bound<'_, PyDict>,
    prefix: Option<&str>,
    step: u64,
    now: i64,
) -> PyResult<()> {
    for (key, value) in data.iter() {
        let key = key.cast::<PyString>().map_err(|_| {
            PyTypeError::new_err(format!("metric names must be str, got {}", type_name(&key)))
        })?;
        let key = key.to_str()?;
        let full;
        let key = match prefix {
            Some(prefix) => {
                full = format!("{prefix}/{key}");
                full.as_str()
            }
            None => key,
        };
        if let Ok(nested) = value.cast::<PyDict>() {
            push_dict(buffer, nested, Some(key), step, now)?;
            continue;
        }
        buffer.push(key, step, now, to_f64(key, &value)?);
    }
    Ok(())
}

fn to_f64(key: &str, value: &Bound<'_, PyAny>) -> PyResult<f64> {
    // Fast paths for the common cases; anything with __float__ (numpy, torch) after.
    if let Ok(float) = value.cast_exact::<PyFloat>() {
        return Ok(float.value());
    }
    if value.is_instance_of::<PyInt>() || value.is_instance_of::<PyBool>() {
        return value.extract::<f64>();
    }
    if value.is_instance_of::<PyString>() || !value.hasattr("__float__")? {
        return Err(PyTypeError::new_err(format!(
            "cannot log {key:?}: {} is not a number (only numbers can be logged for now)",
            type_name(value)
        )));
    }
    value.extract::<f64>()
}

fn type_name(value: &Bound<'_, PyAny>) -> String {
    value
        .get_type()
        .name()
        .map(|n| n.to_string())
        .unwrap_or_else(|_| "?".into())
}

fn parse_object(json: &str) -> PyResult<Map<String, Value>> {
    match serde_json::from_str(json) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(PyValueError::new_err("expected a JSON object")),
        Err(err) => Err(PyValueError::new_err(err.to_string())),
    }
}

#[pyfunction]
#[pyo3(signature = (
    *, project, root, server=None, api_key=None, run_id=None, resume=false, name=String::new(),
    group=String::new(), tags=Vec::new(), notes=String::new(), config_json=String::new(),
    python_version=String::new(), sdk_version=String::new(), console=false,
    system_interval=None, python_path=Vec::new(), details_json=String::new(),
))]
#[allow(clippy::too_many_arguments)]
fn start_run(
    py: Python<'_>,
    project: String,
    root: PathBuf,
    server: Option<String>,
    api_key: Option<String>,
    run_id: Option<String>,
    resume: bool,
    name: String,
    group: String,
    tags: Vec<String>,
    notes: String,
    config_json: String,
    python_version: String,
    sdk_version: String,
    console: bool,
    system_interval: Option<f64>,
    python_path: Vec<PathBuf>,
    details_json: String,
) -> PyResult<NativeRun> {
    let mode = match (server, api_key) {
        (Some(server), Some(api_key)) => Mode::Online { server, api_key },
        (None, None) => Mode::Offline,
        _ => return Err(PyValueError::new_err("server and api_key go together")),
    };
    let mut opts = RunOptions::new(mode, root, project);
    opts.run_id = run_id
        .map(|id| {
            id.parse::<RunId>()
                .map_err(|e| PyValueError::new_err(format!("invalid run id {id:?}: {e}")))
        })
        .transpose()?;
    opts.resume = resume;
    opts.name = name;
    opts.group = group;
    opts.tags = tags;
    opts.notes = notes;
    if !config_json.is_empty() {
        opts.config = parse_object(&config_json)?;
    }
    opts.python_version = python_version;
    opts.sdk_version = sdk_version;
    opts.console = console;
    opts.system_interval = system_interval
        .filter(|s| *s > 0.0)
        .map(|s| Duration::from_secs_f64(s.max(1.0)));
    opts.env = EnvExtra {
        python_path,
        details: if details_json.is_empty() {
            Map::new()
        } else {
            parse_object(&details_json)?
        },
    };

    // Resuming may wait on the server.
    let run = py.detach(|| Run::start(opts)).map_err(|err| match err {
        StartError::InUse(_) | StartError::Resume { .. } | StartError::ResumeOffline(_) => {
            PyRuntimeError::new_err(err.to_string())
        }
        StartError::Local { .. } => PyRuntimeError::new_err(err.to_string()),
    })?;
    Ok(NativeRun { run })
}

/// Uploads runs left in `root`. Returns (run id, "uploaded" | "in_use" | "failed", message).
#[pyfunction]
fn sync(
    py: Python<'_>,
    root: PathBuf,
    server: String,
    api_key: String,
) -> PyResult<Vec<(String, String, String)>> {
    let results = py
        .detach(|| losscape_client::sync(&root, &server, &api_key))
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(results
        .into_iter()
        .map(|r| match r.status {
            SyncStatus::Uploaded => (r.run_id, "uploaded".into(), String::new()),
            SyncStatus::InUse => (
                r.run_id,
                "in_use".into(),
                "still open in another process".into(),
            ),
            SyncStatus::Failed(msg) => (r.run_id, "failed".into(), msg),
        })
        .collect())
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", losscape_core::VERSION)?;
    m.add("PROTOCOL_VERSION", losscape_core::PROTOCOL_VERSION)?;
    m.add_function(wrap_pyfunction!(new_run_id, m)?)?;
    m.add_function(wrap_pyfunction!(start_run, m)?)?;
    m.add_function(wrap_pyfunction!(sync, m)?)?;
    m.add_class::<NativeRun>()?;
    Ok(())
}
