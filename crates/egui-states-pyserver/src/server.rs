use std::net::{SocketAddrV4, TcpStream};
use std::sync::{Arc, atomic};
use std::thread;

use tokio::runtime::Builder;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite::Bytes;

use egui_states_core::event::Event;

use crate::sender::MessageSender;
use crate::server_core::start;
use crate::signals::ChangedValues;
use crate::states_server::ValuesList;

// server -------------------------------------------------------

pub(crate) struct Server {
    connected: Arc<atomic::AtomicBool>,
    enabled: Arc<atomic::AtomicBool>,
    sender: MessageSender,
    start_event: Event,
    addr: SocketAddrV4,
}

impl Server {
    pub(crate) fn new(
        sender: MessageSender,
        rx: UnboundedReceiver<Option<Bytes>>,
        connected: Arc<atomic::AtomicBool>,
        values: ValuesList,
        signals: ChangedValues,
        addr: SocketAddrV4,
        version: u64,
        handshake: Option<Vec<u64>>,
    ) -> Self {
        let start_event = Event::new();
        let enabled = Arc::new(atomic::AtomicBool::new(false));

        let obj = Self {
            connected: connected.clone(),
            enabled: enabled.clone(),
            sender: sender.clone(),
            start_event: start_event.clone(),
            addr,
        };

        let runtime = Builder::new_current_thread()
            .thread_name("Server Runtime")
            .enable_io()
            .worker_threads(2)
            .build()
            .unwrap();

        let server_thread = thread::Builder::new().name("Server".to_string());
        let _ = server_thread.spawn(move || {
            runtime.block_on(async move {
                start(
                    sender,
                    rx,
                    connected,
                    enabled,
                    values,
                    signals,
                    start_event,
                    addr,
                    version,
                    handshake,
                )
                .await;
            });
        });

        obj
    }

    pub(crate) fn start(&mut self) {
        if self.enabled.load(atomic::Ordering::Relaxed) {
            return;
        }

        self.enabled.store(true, atomic::Ordering::Relaxed);
        self.start_event.set();
    }

    pub(crate) fn stop(&mut self) {
        if !self.enabled.load(atomic::Ordering::Relaxed) {
            return;
        }

        self.start_event.clear();
        self.enabled.store(false, atomic::Ordering::Relaxed);
        self.disconnect_client();

        // try to connect to the server to unblock the accept call
        let _ = TcpStream::connect(self.addr); // TODO: use localhost?
    }

    pub(crate) fn disconnect_client(&mut self) {
        if self.connected.load(atomic::Ordering::Relaxed) {
            self.connected.store(false, atomic::Ordering::Relaxed);
            self.sender.close();
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        self.enabled.load(atomic::Ordering::Relaxed)
    }
}

// server traits --------------------------------------------------------------
pub(crate) trait SyncTrait: Sync + Send {
    fn sync(&self);
}

pub(crate) trait Acknowledge: Sync + Send {
    fn acknowledge(&self);
}
