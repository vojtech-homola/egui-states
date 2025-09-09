mod event;
mod py_server;
mod pydict;
mod pygraphs;
mod pyimage;
mod pylist;
mod python_convert;
mod pyvalues;
mod sender;
mod server;
mod server_core;
mod signals;
mod states_server;

pub mod build;

pub use egui_states_macros::{pyenum, pystruct};
pub use pyo3;
pub use python_convert::{EnumInit, FromPython, ToPython};
pub use states_server::ServerValuesCreator;

pub fn init_module(
    m: &pyo3::Bound<pyo3::types::PyModule>,
    create_function: fn(&mut states_server::ServerValuesCreator),
) -> pyo3::PyResult<()> {
    use pyo3::prelude::*;

    py_server::CREATE_HOOK.set(create_function).map_err(|_| {
        pyo3::exceptions::PyRuntimeError::new_err("Failed to inicialize state server module.")
    })?;

    m.add_class::<py_server::StateServerCore>()?;

    Ok(())
}
