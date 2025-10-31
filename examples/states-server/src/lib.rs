mod states;

use pyo3::prelude::*;

#[pymodule]
#[pyo3(name = "core")]
fn init_module(m: &Bound<PyModule>) -> PyResult<()> {
    egui_states_server::init_module(m, states::create_states)?;

    Ok(())
}