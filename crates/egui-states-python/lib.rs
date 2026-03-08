use pyo3::prelude::*;
use egui_states::python::init_module;

#[pymodule(gil_used = false)]
#[pyo3(name = "_core")]
fn init_python_module(m: &Bound<PyModule>) -> PyResult<()> {
    init_module(m)
}