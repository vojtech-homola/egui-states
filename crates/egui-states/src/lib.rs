mod channel;
mod client;
mod dict;
mod graphs;
mod image;
mod list;
mod values;

pub use dict::ValueDict;
pub use graphs::ValueGraphs;
pub use image::ValueImage;
pub use list::ValueList;
pub use values::{Diff, Signal, Value, ValueStatic};

pub(crate) trait UpdateValue: Sync + Send {
    fn update_value(&self, data: &[u8]) -> Result<bool, String>;
}
