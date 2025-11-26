use parking_lot::RwLock;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, OnceLock, atomic};

use pyo3::buffer::PyBuffer;
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyDict, PyList, PyTuple};
use tokio_tungstenite::tungstenite::Bytes;

// use egui_states_core::controls::ControlMessage;
use egui_states_core::nohash::NoHashSet;

use crate::sender::MessageSender;
use crate::server::Server;
use crate::signals::ChangedValues;

#[pyclass]
pub struct StateServerCore {
    server: RwLock<Server>,
}

#[pymethods]
impl StateServerCore {
    #[new]
    #[pyo3(signature = (port, ip_addr=None, handshake=None))]
    fn new(port: u16, ip_addr: Option<[u8; 4]>, handshake: Option<Vec<u64>>) -> PyResult<Self> {
        let addr = match ip_addr {
            Some(addr) => {
                SocketAddrV4::new(Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]), port)
            }
            None => SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), port),
        };

        let server = Server::new(addr, handshake);
        Ok(Self {
            server: RwLock::new(server),
        })
    }

    fn initialize(&self) {
        self.server.write().initialize();
    }

    fn start(&self) {
        self.server.write().start();
    }

    fn stop(&self) {
        self.server.write().stop();
    }

    fn disconnect_clients(&self) {
        self.server.write().disconnect_client();
    }

    fn is_running(&self) -> bool {
        self.server.read().is_running()
    }
}
