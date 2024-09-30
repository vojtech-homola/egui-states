mod server;
mod signals;
mod transport;

pub mod dict;
pub mod graphs;
pub mod image;
pub mod list;
pub mod py_convert;
pub mod py_server;
pub mod states_creator;
pub mod values;

pub(crate) trait SyncTrait: Sync + Send {
    fn sync(&self);
}

pub(crate) trait Acknowledge: Sync + Send {
    fn acknowledge(&self);
}
