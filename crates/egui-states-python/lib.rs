//! Python extension-module entry point for the `egui_states._core` module.

use egui_states::python::init_module;
use pyo3::prelude::*;

#[pymodule(gil_used = false)]
#[pyo3(name = "_core")]
/// Registers the native state-server implementation in the Python module.
fn init_python_module(m: &Bound<PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    init_module(m)
}
