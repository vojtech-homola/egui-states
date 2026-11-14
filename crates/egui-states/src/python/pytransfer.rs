//! The Python side of the file transfer endpoints.
//!
//! The public Python classes live in `egui_states/transfer.py`; they wrap the
//! private native classes here, which return plain values.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use pyo3::exceptions::{PyOSError, PyTimeoutError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::file_transfer::engine::Engine;
use crate::file_transfer::shared::TransferShared;
use crate::file_transfer::{TransferError, TransferStatus};
use crate::python::pyserver::{decode_header, encode_header};
use crate::python::pytypes::PyObjectType;
use crate::server_core::file_transfer as core;

/// The Python exception, imported when first raised.
mod py_exception {
    pyo3::import_exception!(egui_states.transfer, TransferError);
}

/// The `TransferErrorKind` value of `error`.
fn kind(error: &TransferError) -> u8 {
    match error {
        TransferError::Busy => 0,
        TransferError::NotConnected => 1,
        TransferError::Rejected(_) => 2,
        TransferError::Cancelled { .. } => 3,
        TransferError::Disconnected => 4,
        TransferError::Io(_) => 5,
        TransferError::Unsupported(_) => 6,
        TransferError::TooLarge => 8,
        _ => 7,
    }
}

/// The exception a failed call raises: builtin exceptions where Python has
/// one, `TransferError` otherwise.
pub(crate) fn to_py_error(error: TransferError) -> PyErr {
    let message = error.to_string();
    match error {
        TransferError::Io(_) => PyOSError::new_err(message),
        TransferError::InvalidInput(_) | TransferError::Meta(_) => PyValueError::new_err(message),
        TransferError::Timeout => PyTimeoutError::new_err(message),
        other => py_exception::TransferError::new_err((message, kind(&other))),
    }
}

fn duration(seconds: Option<f64>) -> PyResult<Option<Duration>> {
    seconds
        .map(|seconds| {
            Duration::try_from_secs_f64(seconds)
                .map_err(|_| PyValueError::new_err("timeout must be a finite number >= 0"))
        })
        .transpose()
}

/// One transfer; `egui_states.transfer.TransferHandle` wraps it.
#[pyclass(frozen, name = "_TransferHandle", module = "egui_states._core")]
pub(crate) struct PyTransferHandle {
    shared: Arc<TransferShared>,
    meta: Option<PyObjectType>,
}

impl PyTransferHandle {
    pub(crate) fn new(
        py: Python,
        shared: Arc<TransferShared>,
        meta: &Option<PyObjectType>,
    ) -> Self {
        Self {
            shared,
            meta: meta.as_ref().map(|meta| meta.clone_py(py)),
        }
    }
}

#[pymethods]
impl PyTransferHandle {
    fn name(&self) -> String {
        self.shared.name().to_string()
    }

    /// 0 pending, 1 running, 2 done, 3 failed.
    fn status(&self) -> u8 {
        match self.shared.status() {
            TransferStatus::Pending => 0,
            TransferStatus::Running => 1,
            TransferStatus::Done => 2,
            TransferStatus::Failed(_) => 3,
        }
    }

    /// The kind and message of the failure.
    fn error(&self) -> Option<(u8, String)> {
        self.shared
            .error()
            .map(|error| (kind(&error), error.to_string()))
    }

    fn progress(&self) -> (u64, Option<u64>, u32, u32) {
        let progress = self.shared.progress();
        (
            progress.done,
            progress.total,
            progress.files_done,
            progress.files_total,
        )
    }

    fn is_finished(&self) -> bool {
        self.shared.is_finished()
    }

    /// Waits without the GIL; returns whether the transfer finished.
    #[pyo3(signature = (timeout=None))]
    fn wait(&self, py: Python, timeout: Option<f64>) -> PyResult<bool> {
        let timeout = duration(timeout)?;
        let shared = self.shared.clone();
        Ok(py.detach(move || shared.wait(timeout)))
    }

    #[pyo3(signature = (reason=None))]
    fn cancel(&self, py: Python, reason: Option<String>) {
        let shared = self.shared.clone();
        py.detach(move || shared.cancel(reason));
    }

    fn path(&self) -> Option<PathBuf> {
        self.shared.path()
    }

    fn take_data<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.shared.take_data().map(|data| PyBytes::new(py, &data))
    }

    fn meta<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        match self.shared.meta() {
            Some(meta) => decode_header(py, meta, &self.meta),
            None => Ok(None),
        }
    }
}

/// What a send needs from the registry.
pub(crate) struct SendTarget<'a> {
    pub(crate) engine: Engine,
    pub(crate) endpoint: u64,
    pub(crate) meta_type: &'a Option<PyObjectType>,
}

/// Encodes the metadata a Python call passed as `(meta,)`; an endpoint
/// without metadata encodes `()`, which is no bytes.
fn encode_meta(meta: Option<(Bound<PyAny>,)>, meta_type: &Option<PyObjectType>) -> PyResult<Bytes> {
    match (meta, meta_type) {
        (_, None) => Ok(Bytes::new()),
        (None, Some(_)) => Err(PyValueError::new_err("this endpoint needs meta")),
        (meta, Some(_)) => Ok(encode_header(meta, meta_type)?.unwrap_or_default()),
    }
}

/// The offer a send makes; built without the GIL, as it reads the files.
pub(crate) enum Offer {
    File(PathBuf, Option<String>),
    Files(Vec<PathBuf>, String, Option<PathBuf>),
    Bytes(Vec<u8>, String),
}

pub(crate) fn send(
    py: Python,
    target: SendTarget,
    offer: Offer,
    meta: Option<(Bound<PyAny>,)>,
    wait: bool,
    timeout: Option<f64>,
) -> PyResult<PyTransferHandle> {
    let meta = encode_meta(meta, target.meta_type)?;
    let timeout = duration(timeout)?;
    let SendTarget {
        engine, endpoint, ..
    } = target;
    let shared = py
        .detach(move || {
            let (body, source) = match offer {
                Offer::File(path, name) => core::file_offer(&path, name.as_deref())?,
                Offer::Files(paths, name, root) => {
                    core::files_offer(&paths, &name, root.as_deref())?
                }
                Offer::Bytes(data, name) => core::bytes_offer(data, &name),
            };
            core::send(&engine, endpoint, body, meta, source, wait, timeout)
        })
        .map_err(to_py_error)?;
    Ok(PyTransferHandle::new(py, shared, target.meta_type))
}
