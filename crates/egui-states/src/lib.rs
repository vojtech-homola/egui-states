mod channel;
mod client;
mod dict;
mod graphs;
mod image;
mod list;
mod states_creator;
mod values;

pub use client::{
    client::ClientBuilder,
    client_state::{ConnectionState, UIState},
};
pub use dict::ValueDict;
pub use graphs::ValueGraphs;
pub use image::ValueImage;
pub use list::ValueList;
pub use states_creator::ValuesCreator;
pub use values::{Diff, Signal, Value, ValueStatic};

pub(crate) trait UpdateValue: Sync + Send {
    fn update_value(&self, data: &[u8]) -> Result<bool, String>;
}
