mod pyserver;

use pyo3::prelude::*;

#[pymodule]
#[pyo3(name = "core")]
fn init_module(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_class::<pyserver::StateServerCore>()?;

    Ok(())
}
