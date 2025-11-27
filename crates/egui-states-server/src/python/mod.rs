mod pyparsing;
mod pyserver;
mod type_creator;

use pyo3::prelude::*;

#[pymodule]
#[pyo3(name = "_core")]
fn init_module(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_class::<pyserver::StateServerCore>()?;
    m.add_class::<type_creator::PyObjectType>()?;

    Ok(())
}
