mod states;

use pyo3::prelude::*;

#[pymodule]
#[pyo3(name = "core")]
fn init_module(m: &Bound<PyModule>) -> PyResult<()> {
    use egui_pysync;

    egui_pysync::init_module(m, states::create_states)?;

    Ok(())
}