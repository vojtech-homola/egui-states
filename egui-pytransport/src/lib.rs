pub mod collections;
pub mod commands;
pub mod dict;
pub mod event;
pub mod graphs;
pub mod image;
pub mod list;
pub mod nohash;
pub mod transport;
pub mod values;
pub mod items;

pub use values::{ReadValue, WriteValue};

// traints for EnumValue -------------------------------------------------------
pub use egui_pymacros::{EnumInt, EnumStr};

pub trait EnumStr: Send + Sync + Copy {
    fn as_str(&self) -> &'static str;
}

pub trait EnumInt: Sized + Send + Sync + Copy {
    fn as_int(&self) -> u64;
    fn from_int(value: u64) -> Result<Self, ()>;
}

pub use nohash::{NoHashMap, NoHashSet};
