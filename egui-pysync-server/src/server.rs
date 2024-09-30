use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::{
    atomic,
    mpsc::{Receiver, Sender},
    Arc,
};
use std::thread::{spawn, JoinHandle};

use egui_pysync_common::commands::CommandMessage;
use egui_pysync_common::event::Event;
use egui_pysync_common::transport::{ParseError, HEAD_SIZE};
use egui_pysync_common::values::ValueMessage;

use crate::signals::ChangedValues;
use crate::states_creator::ValuesList;
use crate::transport::{read_head, read_message, WriteMessage};

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

        let thread = spawn(move || {
            let mut head = [0u8; HEAD_SIZE];
            loop {
                // read the message
                let value = read_head(&mut head, &mut stream);

                // check if not connected
                if !connected.load(atomic::Ordering::Relaxed) {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    break;
                }

                if let Err(e) = value {
                    match e {
                        ParseError::Connection(e) => {
                            let error = format!("Error reading message: {:?}", e);
                            signals.set(0, ValueMessage::String(error));
                            connected.store(false, atomic::Ordering::Relaxed);
                            break;
                        }
                        ParseError::Parse(e) => {
                            let error = format!("Error parsing message: {}", e);
                            signals.set(0, ValueMessage::String(error));
                            continue;
                        }
                    }
                }

                // process posible command message
                if let Some(command) = value.unwrap() {
                    match command {
                        CommandMessage::Ack(v) => {
                            let val_res = values.ack.get(&v);
                            match val_res {
                                Some(val) => val.acknowledge(),
                                None => {
                                    let error =
                                        format!("Value with id {} not found for Ack command", v);
                                    signals.set(0, ValueMessage::String(error));
                                }
                            }
                        }
                        CommandMessage::Error(err) => {
                            let error = format!("Error message from UI client: {}", err);
                            signals.set(0, ValueMessage::String(error));
                        }
                        _ => {
                            let err = format!(
                                "Command {} should not be processed here",
                                command.as_str()
                            );
                            signals.set(0, ValueMessage::String(err));
                        }
                    }
                    continue;
                }

                let result = read_message(&values, &signals, &head, &mut stream);
                if let Err(e) = result {
                    match e {
                        ParseError::Connection(e) => {
                            let error = format!("Error reading message: {:?}", e);
                            signals.set(0, ValueMessage::String(error));
                            connected.store(false, atomic::Ordering::Relaxed);
                            break;
                        }
                        ParseError::Parse(error) => {
                            let text = format!("Error parsing message: {}", error);
                            signals.set(0, ValueMessage::String(text));
                            continue;
                        }
                    }
                }
            }

            // send close signal to writing thread if reading fails
            channel.send(WriteMessage::Terminate).unwrap();

            // wait for writing thread to finish and return the receiver
            writer.join().unwrap()
        });

        Self { thread }
    }

    fn writer(
        rx: Receiver<WriteMessage>,
        connected: Arc<AtomicBool>,
        mut stream: TcpStream,
        signals: ChangedValues,
    ) -> JoinHandle<Receiver<WriteMessage>> {
        spawn(move || {
            let mut head = [0u8; HEAD_SIZE];

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
                let res = message.write_message(&mut head, &mut stream);
                if let Err(e) = res {
                    let error = format!("Error writing message: {:?}", e);
                    signals.set(0, ValueMessage::String(error));
                    connected.store(false, atomic::Ordering::Relaxed);
                    break;
                }
            }
            rx
        })
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
}

impl Server {
    pub(crate) fn new(
        channel: Sender<WriteMessage>,
        rx: Receiver<WriteMessage>,
        connected: Arc<atomic::AtomicBool>,
        values: ValuesList,
        signals: ChangedValues,
    ) -> Self {
        let start_event = Event::new();
        let enabled = Arc::new(atomic::AtomicBool::new(false));

        let obj = Self {
            connected: connected.clone(),
            enabled: enabled.clone(),
            channel: channel.clone(),
            start_event: start_event.clone(),
        };

        spawn(move || {
            let mut holder = ChannelHolder::Rx(rx);

            loop {
                // wait for start control event
                start_event.wait();

                // listen to incoming connections
                let listener = TcpListener::bind("127.0.0.1:888");
                if let Err(e) = listener {
                    println!("Error binding: {:?}", e); // TODO: log error
                    continue;
                }
                let listener = listener.unwrap();

                // accept incoming connection
                let stream = listener.accept();
                if stream.is_err() {
                    println!("Error accepting connection"); // TODO: log error
                    continue;
                }
                let mut stream = stream.unwrap().0;

                // if server is disabled, go back and wait for start control event
                if !enabled.load(atomic::Ordering::Relaxed) {
                    stream.shutdown(std::net::Shutdown::Both).unwrap();
                    continue;
                }

                // read the message
                let mut head = [0u8; HEAD_SIZE];
                let value = read_head(&mut head, &mut stream);
                if let Err(e) = value {
                    match e {
                        ParseError::Connection(e) => {
                            let error = format!("Error reading message: {:?}", e);
                            signals.set(0, ValueMessage::String(error));
                            connected.store(false, atomic::Ordering::Relaxed);
                            break;
                        }
                        ParseError::Parse(e) => {
                            let error = format!("Error parsing message: {}", e);
                            signals.set(0, ValueMessage::String(error));
                            continue;
                        }
                    }
                }

                // check if message is handshake
                if let Some(CommandMessage::Handshake(_)) = value.unwrap() {
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
