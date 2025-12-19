mod pygraphs;
mod pyimage;
mod pyparsing;
mod pyserver;
mod pytypes;

use pyo3::prelude::*;

#[pymodule(gil_used = false)]
#[pyo3(name = "_core")]
fn init_module(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_class::<pyserver::StateServerCore>()?;
    m.add_class::<pytypes::PyObjectType>()?;

    m.add("u8", pytypes::U8)?;
    m.add("u16", pytypes::U16)?;
    m.add("u32", pytypes::U32)?;
    m.add("u64", pytypes::U64)?;
    m.add("i8", pytypes::I8)?;
    m.add("i16", pytypes::I16)?;
    m.add("i32", pytypes::I32)?;
    m.add("i64", pytypes::I64)?;
    m.add("f32", pytypes::F32)?;
    m.add("f64", pytypes::F64)?;
    m.add("bo", pytypes::BO)?;
    m.add("st", pytypes::STR)?;
    m.add("emp", pytypes::EMP)?;

    m.add_function(pyo3::wrap_pyfunction!(pytypes::opt, m)?)?;
    m.add_function(pyo3::wrap_pyfunction!(pytypes::tu, m)?)?;
    m.add_function(pyo3::wrap_pyfunction!(pytypes::cl, m)?)?;
    m.add_function(pyo3::wrap_pyfunction!(pytypes::li, m)?)?;
    m.add_function(pyo3::wrap_pyfunction!(pytypes::vec, m)?)?;
    m.add_function(pyo3::wrap_pyfunction!(pytypes::map, m)?)?;
    m.add_function(pyo3::wrap_pyfunction!(pytypes::enu, m)?)?;

    Ok(())
}
