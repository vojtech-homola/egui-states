mod client;
mod client_state;
mod dict;
mod graphs;
mod handle_message;
mod image;
mod list;
mod sender;
mod states_creator;
mod values;

pub use client::ClientBuilder;
pub use client_state::{ConnectionState, UIState};
pub use dict::ValueDict;
pub use graphs::ValueGraphs;
pub use image::ValueImage;
pub use list::ValueList;
pub use states_creator::ValuesCreator;
pub use values::{Diff, Signal, Value, ValueStatic};

pub trait UpdateValue: Sync + Send {
    fn update_value(&self, data: &[u8]) -> Result<bool, String>;
}

pub trait State {
    fn new(c: &mut ValuesCreator) -> Self;
}
