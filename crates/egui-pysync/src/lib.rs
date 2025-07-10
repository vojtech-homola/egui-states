pub mod build;

pub mod client;
pub mod client_state;
pub mod dict;
pub mod graphs;
pub mod image;
pub mod list;
pub mod values;

mod commands;
mod event;
mod nohash;
mod states_creator;
mod transport;

#[cfg(feature = "server")]
mod py_server;
#[cfg(feature = "server")]
mod python_convert;
#[cfg(feature = "server")]
mod server;
#[cfg(feature = "server")]
mod signals;
#[cfg(feature = "server")]
mod states_server;

pub use dict::ValueDict;
pub use graphs::ValueGraphs;
pub use image::ValueImage;
pub use list::ValueList;
pub use states_creator::ValuesCreator;
pub use values::{Diff, Empty, Signal, Value, ValueStatic};

pub use serde;

// python -----------------------------------------------------------------------
#[cfg(feature = "server")]
pub use egui_pysync_macros::{pyenum, pystruct};

#[cfg(feature = "server")]
pub use states_server::ServerValuesCreator;

#[cfg(feature = "server")]
pub use python_convert::ToPython;

#[cfg(feature = "server")]
pub use crate::python_convert::EnumInit;

// nohash -----------------------------------------------------------------------
pub use nohash::{NoHashMap, NoHashSet};

// server -----------------------------------------------------------------------
#[cfg(feature = "server")]
pub use pyo3;

#[cfg(feature = "server")]
use pyo3::prelude::*;

#[cfg(feature = "server")]
pub fn init_module(
    m: &pyo3::Bound<pyo3::types::PyModule>,
    create_function: fn(&mut states_server::ServerValuesCreator),
) -> pyo3::PyResult<()> {
    py_server::CREATE_HOOK.set(create_function).map_err(|_| {
        pyo3::exceptions::PyRuntimeError::new_err("Failed to inicialize state server module.")
    })?;

    m.add_class::<py_server::StateServerCore>()?;

    Ok(())
}
