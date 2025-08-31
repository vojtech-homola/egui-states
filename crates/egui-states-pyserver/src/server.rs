use std::net::{SocketAddrV4, TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::{
    atomic,
    mpsc::{Receiver, Sender},
    Arc,
};
use std::thread::{self, JoinHandle};

use egui_states_core::event::Event;
use egui_states_core::controls::ControlMessage;

use crate::signals::ChangedValues;
use crate::states_server::ValuesList;
use crate::transport::{read_message, write_message, ReadMessage, WriteMessage};

struct StatesTransfer {
    thread: JoinHandle<Receiver<WriteMessage>>,
}

impl StatesTransfer {
    fn start(
        connected: Arc<AtomicBool>,
        values: ValuesList,
        signals: ChangedValues,
        mut stream: TcpStream,
        rx: Receiver<WriteMessage>,
        channel: Sender<WriteMessage>,
    ) -> Self {
        let writer = Self::writer(
            rx,
            connected.clone(),
            stream.try_clone().unwrap(),
            signals.clone(),
        );

        let read_thread = thread::Builder::new().name("Reader".to_string());
        let thread = read_thread
            .spawn(move || {
                loop {
                    // read the message
                    let res = read_message(&mut stream);

                    // check if not connected
                    if !connected.load(atomic::Ordering::Relaxed) {
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                        break;
                    }

                    if let Err(e) = res {
                        let error = format!("Error reading message: {:?}", e);
                        signals.set(0, error);
                        connected.store(false, atomic::Ordering::Relaxed);
                        break;
                    }
                    let message = res.unwrap();

                    // process posible command message
                    if let ReadMessage::Command(command) = message {
                        match command {
                            ControlMessage::Ack(v) => {
                                let val_res = values.ack.get(&v);
                                match val_res {
                                    Some(val) => val.acknowledge(),
                                    None => {
                                        let error = format!(
                                            "Value with id {} not found for Ack command",
                                            v
                                        );
                                        signals.set(0, error);
                                    }
                                }
                            }
                            ControlMessage::Error(err) => {
                                let error = format!("Error message from UI client: {}", err);
                                signals.set(0, error);
                            }
                            _ => {
                                let err = format!(
                                    "Command {} should not be processed here",
                                    command.as_str()
                                );
                                signals.set(0, err);
                            }
                        }
                        continue;
                    }

                    // process message
                    let res = match message {
                        ReadMessage::Value(id, signal, data) => match values.updated.get(&id) {
                            Some(val) => val.update_value(data, signal),
                            None => Err(format!("Value with id {} not found", id)),
                        },

                        ReadMessage::Signal(id, data) => match values.updated.get(&id) {
                            Some(val) => val.update_value(data, true),
                            None => Err(format!("Value with id {} not found", id)),
                        },

                        _ => Err(format!(
                            "Message {} should not be processed here",
                            message.to_str()
                        )),
                    };

                    if let Err(e) = res {
                        let text = format!("Error processing message: {}", e);
                        signals.set(0, text);
                    }
                }

                // send close signal to writing thread if reading fails
                channel.send(WriteMessage::Terminate).unwrap();

                // wait for writing thread to finish and return the receiver
                writer.join().unwrap()
            })
            .unwrap();

        Self { thread }
    }

    fn writer(
        rx: Receiver<WriteMessage>,
        connected: Arc<AtomicBool>,
        mut stream: TcpStream,
        signals: ChangedValues,
    ) -> JoinHandle<Receiver<WriteMessage>> {
        let thread = thread::Builder::new().name("Writer".to_string());
        thread
            .spawn(move || {
                loop {
                    // get message from channel
                    let message = rx.recv().unwrap();

                    // check if message is terminate signal
                    if let WriteMessage::Terminate = message {
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                        break;
                    }

                    // if not connected, stop thread
                    if !connected.load(atomic::Ordering::Relaxed) {
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                        break;
                    }

                    // send message
                    let res = write_message(message, &mut stream);
                    if let Err(e) = res {
                        let error = format!("Error writing message: {:?}", e);
                        signals.set(0, error);
                        connected.store(false, atomic::Ordering::Relaxed);
                        break;
                    }
                }
                rx
            })
            .unwrap()
    }

    fn join(self) -> Receiver<WriteMessage> {
        self.thread.join().unwrap()
    }
}

// server -------------------------------------------------------
enum ChannelHolder {
    Transfer(StatesTransfer),
    Rx(Receiver<WriteMessage>),
}

pub(crate) struct Server {
    connected: Arc<atomic::AtomicBool>,
    enabled: Arc<atomic::AtomicBool>,
    channel: Sender<WriteMessage>,
    start_event: Event,
    addr: SocketAddrV4,
}

impl Server {
    pub(crate) fn new(
        channel: Sender<WriteMessage>,
        rx: Receiver<WriteMessage>,
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
            channel: channel.clone(),
            start_event: start_event.clone(),
            addr,
        };

        let server_thread = thread::Builder::new().name("Server".to_string());
        let _ = server_thread.spawn(move || {
            let mut holder = ChannelHolder::Rx(rx);

            loop {
                // wait for start control event
                start_event.wait();

                // listen to incoming connections
                let listener = TcpListener::bind(addr);
                if let Err(e) = listener {
                    let error = format!("Error binding: {:?}", e);
                    signals.set(0, error);
                    continue;
                }
                let listener = listener.unwrap();

                // accept incoming connection
                let stream = listener.accept();

                // if server is disabled, go back and wait for start control event
                if !enabled.load(atomic::Ordering::Relaxed) {
                    if let Ok((stream, _)) = stream {
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                    }
                    continue;
                }

                // check if error accepting connection
                if let Err(e) = stream {
                    let error = format!("Error accepting connection: {:?}", e);
                    signals.set(0, error);
                    continue;
                }
                let mut stream = stream.unwrap().0;

                // read the message
                let res = read_message(&mut stream);
                if let Err(e) = res {
                    let error = format!("Error reading initial message: {:?}", e);
                    signals.set(0, error);
                    connected.store(false, atomic::Ordering::Relaxed);
                    continue;
                }

                // check if message is handshake
                if let ReadMessage::Command(ControlMessage::Handshake(v, h)) = res.unwrap() {
                    if v != version {
                        let error = format!("Attempted to connect with different version: {}, version {} is required.", v, version);
                        signals.set(0, error);
                        continue;
                    }

                    if let Some(ref hash) = handshake {
                        if !hash.contains(&h) {
                            let error = "Attempted to connect with wrong hash".to_string();
                            signals.set(0, error);
                            continue;
                        }
                    }

                    let rx = match holder {
                        // disconnect previous client
                        ChannelHolder::Transfer(st) => {
                            connected.store(false, atomic::Ordering::Relaxed);
                            channel.send(WriteMessage::Terminate).unwrap();
                            st.join()
                        }
                        ChannelHolder::Rx(rx) => rx,
                    };

                    connected.store(true, atomic::Ordering::Relaxed);

                    // clean mesage queue and send sync signals
                    for _v in rx.try_iter() {}
                    for (_, v) in values.sync.iter() {
                        v.sync();
                    }

                    // start transfer thread
                    let st_transfer = StatesTransfer::start(
                        connected.clone(),
                        values.clone(),
                        signals.clone(),
                        stream,
                        rx,
                        channel.clone(),
                    );
                    holder = ChannelHolder::Transfer(st_transfer);
                }
            }
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
        let _ = TcpStream::connect(self.addr);
    }

    pub(crate) fn disconnect_client(&mut self) {
        if self.connected.load(atomic::Ordering::Relaxed) {
            self.connected.store(false, atomic::Ordering::Relaxed);
            self.channel.send(WriteMessage::Terminate).unwrap();
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
