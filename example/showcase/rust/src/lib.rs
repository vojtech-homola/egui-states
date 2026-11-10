//! Setup shared by the runnable showcase and its smoke tests.
use egui_states::server as s;
use std::collections::HashMap;
include!(concat!(env!("OUT_DIR"), "/bindings_module.rs"));
use bindings::StatesServer;

pub const DEFAULT_VEC: [i32; 3] = [10, -3, 27];
pub fn default_map() -> HashMap<u16, u32> {
    HashMap::from([(1, 100), (2, 200), (5, 500)])
}

pub struct ShowcaseServer {
    pub server: StatesServer,
    // Dropping callback handles disconnects them.
    _callbacks: Vec<s::CallbackHandle>,
}

impl Drop for ShowcaseServer {
    fn drop(&mut self) {
        self.server.stop();
    }
}

pub fn setup_server() -> s::Result<ShowcaseServer> {
    setup_server_with_options(s::ServerOptions::new())
}

/// Like [`setup_server`], with explicit options such as a web page directory.
pub fn setup_server_with_options(options: s::ServerOptions) -> s::Result<ShowcaseServer> {
    let server = StatesServer::with_options(options)?;
    let callbacks = register_callbacks(&server);
    populate_initial_data(&server.states)?;
    Ok(ShowcaseServer {
        server,
        _callbacks: callbacks,
    })
}

mod callbacks;
mod initial_data;
pub use callbacks::register_callbacks;
pub use initial_data::populate_initial_data;
