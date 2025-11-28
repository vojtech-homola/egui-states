mod pygraphs;
mod pyimage;
mod pyparsing;
mod pyserver;
mod pytypes;

use pyo3::prelude::*;

#[pymodule]
#[pyo3(name = "_core")]
fn init_module(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_class::<pyserver::StateServerCore>()?;
    m.add_class::<pytypes::PyObjectType>()?;

    Ok(())
}
