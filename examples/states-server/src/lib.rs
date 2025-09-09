mod states;

use pyo3::prelude::*;

#[pymodule]
#[pyo3(name = "core")]
fn init_module(m: &Bound<PyModule>) -> PyResult<()> {
    use egui_states_pyserver;

    egui_states_pyserver::init_module(m, states::create_states)?;

    Ok(())
}