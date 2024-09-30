pub mod collections;
pub mod commands;
pub mod event;
pub mod image;
pub mod transport;
pub mod values;

// pub use transport::ParseError;
// pub use values::{ReadValue, WriteValue};

// traints for EnumValue -------------------------------------------------------
// pub use states_server_macros::{EnumInt, EnumStr};

pub trait EnumStr: Send + Sync + Copy {
    fn as_str(&self) -> &'static str;
}

pub trait EnumInt: Sized + Send + Sync + Copy {
    fn as_int(&self) -> u64;
    fn from_int(value: u64) -> Result<Self, ()>;
}
