use std::net::{SocketAddrV4, TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::{
    Arc, atomic,
    mpsc::{Receiver, Sender},
};
use std::thread::{self, JoinHandle};

use tungstenite::{self, Bytes, Message};

use egui_states_core::controls::ControlMessage;
use egui_states_core::event::Event;
use egui_states_core::serialization;

use crate::signals::ChangedValues;
use crate::states_server::ValuesList;
// use crate::transport::{ReadMessage, WriteMessage, read_message, write_message};

struct StatesTransfer {
    thread: JoinHandle<Receiver<Option<Bytes>>>,
}

impl StatesTransfer {
    fn start(
        connected: Arc<AtomicBool>,
        values: ValuesList,
        signals: ChangedValues,
        stream: TcpStream,
        rx: Receiver<Option<Bytes>>,
        channel: Sender<Option<Bytes>>,
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
                let mut websocket = tungstenite::WebSocket::from_raw_socket(
                    stream.try_clone().unwrap(),
                    tungstenite::protocol::Role::Server,
                    None,
                );
                loop {
                    // read the message
                    let res = websocket.read();

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

                    let res = match message {
                        Message::Binary(m) => {
                            let data = m.as_ref();
                            match data[0] {
                                serialization::TYPE_CONTROL => {
                                    let control = ControlMessage::deserialize(data).unwrap(); //TODO handle error
                                    match control {
                                        ControlMessage::Ack(v) => {
                                            let val_res = values.ack.get(&v);
                                            match val_res {
                                                Some(val) => {
                                                    val.acknowledge();
                                                    Ok(())
                                                }
                                                None => Err(format!(
                                                    "Value with id {} not found for Ack command",
                                                    v
                                                )),
                                            }
                                        }
                                        ControlMessage::Error(err) => {
                                            Err(format!("Error message from UI client: {}", err))
                                        }
                                        _ => Err(format!(
                                            "Command {} should not be processed here",
                                            control.as_str()
                                        )),
                                    }
                                    // continue;
                                }
                                serialization::TYPE_VALUE => {
                                    let id =
                                        u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                                    match values.updated.get(&id) {
                                        Some(val) => val.update_value(&data[5..]),
                                        None => Err(format!("Value with id {} not found", id)),
                                    }
                                }
                                serialization::TYPE_SIGNAL => {
                                    let id =
                                        u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                                    match values.updated.get(&id) {
                                        Some(val) => val.update_value(&data[5..]),
                                        None => Err(format!("Value with id {} not found", id)),
                                    }
                                }
                                _ => Err(format!("Unexpected message type: {}", data[0])),
                            }
                        }
                        _ => Err("Unexpected message format".to_string()),
                    };

                    if let Err(e) = res {
                        let text = format!("Error processing message: {}", e);
                        signals.set(0, text);
                    }
                }

                // send close signal to writing thread if reading fails
                channel.send(None).unwrap();

                // wait for writing thread to finish and return the receiver
                writer.join().unwrap()
            })
            .unwrap();

        Self { thread }
    }

    fn writer(
        rx: Receiver<Option<Bytes>>,
        connected: Arc<AtomicBool>,
        stream: TcpStream,
        signals: ChangedValues,
    ) -> JoinHandle<Receiver<Option<Bytes>>> {
        let thread = thread::Builder::new().name("Writer".to_string());
        thread
            .spawn(move || {
                let mut websocket = tungstenite::accept(stream).unwrap();
                loop {
                    // get message from channel
                    let message = rx.recv().unwrap();

                    // check if message is terminate signal
                    if message.is_none() {
                        let _ = websocket.get_mut().shutdown(std::net::Shutdown::Both);
                        break;
                    }
                    let message = message.unwrap();

                    // if not connected, stop thread
                    if !connected.load(atomic::Ordering::Relaxed) {
                        let _ = websocket.get_mut().shutdown(std::net::Shutdown::Both);
                        break;
                    }

                    // send message
                    let res = websocket.write(Message::Binary(message));
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

    fn join(self) -> Receiver<Option<Bytes>> {
        self.thread.join().unwrap()
    }
}

// server -------------------------------------------------------
enum ChannelHolder {
    Transfer(StatesTransfer),
    Rx(Receiver<Option<Bytes>>),
}

pub(crate) struct Server {
    connected: Arc<atomic::AtomicBool>,
    enabled: Arc<atomic::AtomicBool>,
    channel: Sender<Option<Bytes>>,
    start_event: Event,
    addr: SocketAddrV4,
}

impl Server {
    pub(crate) fn new(
        channel: Sender<Option<Bytes>>,
        rx: Receiver<Option<Bytes>>,
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
                let stream = stream.unwrap().0;
                let mut websocket = tungstenite::accept(stream.try_clone().unwrap()).unwrap(); //TODO: handle errors

                // read the message
                let res = websocket.read();
                if let Err(e) = res {
                    let error = format!("Error reading initial message: {:?}", e);
                    signals.set(0, error);
                    connected.store(false, atomic::Ordering::Relaxed);
                    continue;
                }
                let res = res.unwrap();
                if let Message::Binary(message) = res {
                    let data = message.as_ref();
                    if data[0] == serialization::TYPE_CONTROL {
                        let control = ControlMessage::deserialize(data).unwrap(); //TODO handle error
                        if let ControlMessage::Handshake(v, h) = control {
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
                            channel.send(None).unwrap();
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
                }

                // check if message is handshake
                // if let ReadMessage::Command(ControlMessage::Handshake(v, h)) = res.unwrap() {
                //     if v != version {
                //         let error = format!("Attempted to connect with different version: {}, version {} is required.", v, version);
                //         signals.set(0, error);
                //         continue;
                //     }

                //     if let Some(ref hash) = handshake {
                //         if !hash.contains(&h) {
                //             let error = "Attempted to connect with wrong hash".to_string();
                //             signals.set(0, error);
                //             continue;
                //         }
                //     }

                //     let rx = match holder {
                //         // disconnect previous client
                //         ChannelHolder::Transfer(st) => {
                //             connected.store(false, atomic::Ordering::Relaxed);
                //             channel.send(None).unwrap();
                //             st.join()
                //         }
                //         ChannelHolder::Rx(rx) => rx,
                //     };

                //     connected.store(true, atomic::Ordering::Relaxed);

                //     // clean mesage queue and send sync signals
                //     for _v in rx.try_iter() {}
                //     for (_, v) in values.sync.iter() {
                //         v.sync();
                //     }

                //     // start transfer thread
                //     let st_transfer = StatesTransfer::start(
                //         connected.clone(),
                //         values.clone(),
                //         signals.clone(),
                //         stream,
                //         rx,
                //         channel.clone(),
                //     );
                //     holder = ChannelHolder::Transfer(st_transfer);
                // }
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
        let _ = TcpStream::connect(self.addr); // TODO: use localhost?
    }

    pub(crate) fn disconnect_client(&mut self) {
        if self.connected.load(atomic::Ordering::Relaxed) {
            self.connected.store(false, atomic::Ordering::Relaxed);
            self.channel.send(None).unwrap();
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
