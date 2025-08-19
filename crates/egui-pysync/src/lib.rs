pub mod build;

mod commands;
mod event;
mod nohash;
mod transport;
mod values_common;

pub use nohash::{NoHashMap, NoHashSet};
pub use serde;
pub use values_common::Empty;

// server -----------------------------------------------------------------------
#[cfg(feature = "server")]
mod pyvalues;
#[cfg(feature = "server")]
mod server;

// python
#[cfg(feature = "server")]
pub use egui_pysync_macros::{pyenum, pystruct};
#[cfg(feature = "server")]
pub use pyo3;
#[cfg(feature = "server")]
pub use server::{
    python_convert::{EnumInit, ToPython},
    states_server::ServerValuesCreator,
};

#[cfg(feature = "server")]
pub fn init_module(
    m: &pyo3::Bound<pyo3::types::PyModule>,
    create_function: fn(&mut server::states_server::ServerValuesCreator),
) -> pyo3::PyResult<()> {
    use pyo3::prelude::*;

    server::py_server::CREATE_HOOK
        .set(create_function)
        .map_err(|_| {
            pyo3::exceptions::PyRuntimeError::new_err("Failed to inicialize state server module.")
        })?;

    m.add_class::<server::py_server::StateServerCore>()?;

    Ok(())
}

// client -----------------------------------------------------------------------
#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod values;

#[cfg(feature = "client")]
pub use client::{
    client::ClientBuilder,
    client_state::{ConnectionState, UIState},
    states_creator::ValuesCreator,
};
#[cfg(feature = "client")]
pub use values::{
    dict::ValueDict,
    graphs::ValueGraphs,
    image::ValueImage,
    list::ValueList,
    values::{Diff, Signal, Value, ValueStatic},
};
