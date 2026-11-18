//! The connect request handed to the Python connect-request callback.

use std::sync::Arc;

use pyo3::prelude::*;

use crate::server_core::admission::Request;

/// Native state of a client asking to connect; see
/// `egui_states.ConnectRequest`.
#[pyclass(frozen, name = "_ConnectRequest", module = "egui_states._core")]
pub(crate) struct PyConnectRequest(Arc<Request>);

impl PyConnectRequest {
    pub(crate) fn new(request: Arc<Request>) -> Self {
        Self(request)
    }
}

#[pymethods]
impl PyConnectRequest {
    fn address(&self) -> &str {
        self.0.address()
    }

    fn is_pending(&self) -> bool {
        self.0.is_pending()
    }

    /// Answers the request; only the first answer to a pending one counts.
    fn answer(&self, accept: bool) {
        self.0.answer(accept);
    }
}
