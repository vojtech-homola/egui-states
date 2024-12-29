pub mod build;
pub mod commands;
pub mod dict;
pub mod event;
pub mod graphs;
pub mod image;
pub mod list;
pub mod nohash;
pub mod python_convert;
pub mod signals;
pub mod transport;
pub mod values;

pub mod client;
pub mod server;

mod client_state;
mod py_server;
mod states_creator;
mod states_server;

use pyo3::prelude::*;

pub use values::{Signal, Value, ValueStatic};

// traints for EnumValue -------------------------------------------------------
// pub use egui_pymacros::{EnumImpl, EnumStr};

pub trait EnumStr: Send + Sync + Copy {
    fn as_str(&self) -> &'static str;
}

pub trait EnumInt: Sized + Send + Sync + Copy {
    fn as_int(&self) -> u64;
    fn from_int(value: u64) -> Result<Self, ()>;
}

// nohash -----------------------------------------------------------------------
pub use nohash::{NoHashMap, NoHashSet};

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
