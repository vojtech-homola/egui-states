//! Minimal counter server setup, reusable without opening a socket.
use egui_states::server as s;
include!(concat!(env!("OUT_DIR"), "/bindings_module.rs"));

pub struct CounterServer {
    pub server: bindings::StatesServer,
    _callbacks: Vec<s::CallbackHandle>,
}

impl Drop for CounterServer {
    fn drop(&mut self) {
        self.server.stop();
    }
}

pub fn setup_server() -> s::Result<CounterServer> {
    let server = bindings::StatesServer::new()?;
    server.states.count.set(0)?;
    let log = server
        .states
        .count
        .connect(|value| println!("count: {value}"));
    let count = server.states.count.clone();
    let reset = server.states.reset.connect_empty(move || {
        if let Err(error) = count.set(0) {
            eprintln!("reset failed: {error}");
        }
    });
    Ok(CounterServer {
        server,
        _callbacks: vec![log, reset],
    })
}
