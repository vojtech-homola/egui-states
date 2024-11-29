use pyo3::prelude::*;

mod py_server;
mod server;
mod signals;

pub mod dict;
pub mod graphs;
pub mod image;
pub mod list;
pub mod python_convert;
pub mod states_creator;
pub mod values;

pub(crate) trait SyncTrait: Sync + Send {
    fn sync(&self);
}

pub(crate) trait Acknowledge: Sync + Send {
    fn acknowledge(&self);
}

pub fn init_module(
    m: &Bound<PyModule>,
    create_function: fn(&mut states_creator::ValuesCreator),
) -> PyResult<()> {
    py_server::CREATE_HOOK.set(create_function).map_err(|_| {
        pyo3::exceptions::PyRuntimeError::new_err("Failed to inicialize state server module.")
    })?;

    m.add_class::<py_server::StateServerCore>()?;

    Ok(())
}

pub use dict::ValueDict;
pub use graphs::ValueGraphs;
pub use image::ValueImage;
pub use list::ValueList;
pub use python_convert::ToPython;
pub use states_creator::ValuesCreator;
pub use values::{Signal, Value, ValueStatic};
