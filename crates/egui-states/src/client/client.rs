use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use egui::Context;
use tungstenite::{Bytes, Message};

use egui_states_core::controls::ControlMessage;
use egui_states_core::serialization::{self, MessageData};

use crate::channel::ChannelMessage;
use crate::client::client_state::{ConnectionState, UIState};
use crate::states_creator::{ValuesCreator, ValuesList};

struct ClientChannel {
    sender: Sender<Option<MessageData>>,
}

impl ClientChannel {
    pub fn new(sender: Sender<Option<MessageData>>) -> Arc<Self> {
        Arc::new(Self {
            sender: sender.clone(),
        })
    }
}

impl ChannelMessage for ClientChannel {
    #[inline]
    fn send(&self, message: MessageData) {
        self.sender.send(Some(message)).unwrap();
    }

    #[inline]
    fn close(&self) {
        self.sender.send(None).unwrap();
    }
}

fn handle_message(message: Bytes, vals: &ValuesList, ui_state: &UIState) -> Result<(), String> {
    let data = message.as_ref();
    let message_type = data[0];

    if message_type == serialization::TYPE_CONTROL {
        let control = ControlMessage::deserialize(data)?;
        match control {
            ControlMessage::Update(t) => {
                ui_state.update(t);
            }
            _ => {}
        }
        return Ok(());
    }

    // if let ReadMessage::Command(ref command) = message {
    //     match command {
    //         CommandMessage::Update(t) => {
    //             ui_state.update(*t);
    //         }
    //         _ => {}
    //     }
    //     return Ok(());
    // }

    let id = u32::from_le_bytes(data[1..5].try_into().unwrap());
    let update = match message_type {
        serialization::TYPE_VALUE => match vals.values.get(&id) {
            Some(value) => value.update_value(&data[5..])?,
            None => return Err(format!("Value with id {} not found", id)),
        },

        serialization::TYPE_STATIC => match vals.static_values.get(&id) {
            Some(value) => value.update_value(&data[5..])?,
            None => return Err(format!("Static with id {} not found", id)),
        },

        serialization::TYPE_IMAGE => match vals.images.get(&id) {
            Some(value) => value.update_value(&data[5..])?,
            None => return Err(format!("Image with id {} not found", id)),
        },

        serialization::TYPE_DICT => match vals.dicts.get(&id) {
            Some(value) => value.update_value(&data[5..])?,
            None => return Err(format!("Dict with id {} not found", id)),
        },

        serialization::TYPE_LIST => match vals.lists.get(&id) {
            Some(value) => value.update_value(&data[5..])?,
            None => return Err(format!("List with id {} not found", id)),
        },

        serialization::TYPE_GRAPH => match vals.graphs.get(&id) {
            Some(value) => value.update_value(&data[5..])?,
            None => return Err(format!("Graph with id {} not found", id)),
        },

        _ => return Err(format!("Unknown message type: {}", message_type)),
    };

    if update {
        ui_state.update(0.);
    }

    Ok(())
}

fn start_gui_client(
    addr: SocketAddrV4,
    vals: ValuesList,
    version: u64,
    mut rx: Receiver<Option<MessageData>>,
    channel: Sender<Option<MessageData>>,
    ui_state: UIState,
    handshake: u64,
) {
    let client_thread = thread::Builder::new().name("Client".to_string());
    let _ = client_thread.spawn(move || {
        loop {
            // wait for the connection signal
            ui_state.wait_connection();
            ui_state.set_state(ConnectionState::NotConnected);

            // try to connect to the server
            let address = format!("ws://{}", addr);
            let res = tungstenite::connect(address);
            if res.is_err() {
                continue;
            }

            // get the stream
            let (mut socket_read, _) = res.unwrap();
            // let stream = match socket_read.get_mut() {
            //     tungstenite::stream::MaybeTlsStream::Plain(s) => s,
            //     _ => panic!("TLS is not supported"),
            // };

            // let mut socket_write = tungstenite::WebSocket::from_raw_socket(
            //     stream.try_clone().unwrap(),
            //     tungstenite::protocol::Role::Client,
            //     None,
            // );

            let handshake = ControlMessage::Handshake(version, handshake);
            let message = Message::Binary(Bytes::from(handshake.serialize()));
            let res = socket_read.send(message);
            if let Err(e) = res {
                println!("Error for sending handshake: {:?}", e); // TODO: log error
                return rx;
            }

            // clean message queue before starting
            for _v in rx.try_iter() {}

            // read thread -----------------------------------------
            let th_vals = vals.clone();
            let th_ui_state = ui_state.clone();
            let th_channel = channel.clone();

            let read_thread = thread::Builder::new().name("Read".to_string());
            let recv_tread = read_thread
                .spawn(move || {
                    loop {
                        // read the message
                        println!("can read {}", socket_read.can_read());
                        let res = socket_read.read();
                        if let Err(e) = res {
                            println!("Error reading message: {:?}", e); // TODO: log error
                            break;
                        }
                        let message = res.unwrap();
                        let mess = match message {
                            tungstenite::Message::Binary(d) => d,
                            tungstenite::Message::Close(_) => break,
                            _ => {
                                println!("Wrong type of message received: {:?}", message); // TODO: log error
                                break;
                            }
                        };

                        // handle the message
                        let res = handle_message(mess, &th_vals, &th_ui_state);
                        if let Err(e) = res {
                            let error = format!("Error handling message: {:?}", e);
                            th_channel.send(Some(ControlMessage::error(error))).unwrap();
                            break;
                        }
                    }
                })
                .unwrap();

            // send thread -----------------------------------------
            let write_thread = thread::Builder::new().name("Write".to_string());
            let send_thread = write_thread
                .spawn(move || {
                    // send handshake
                    // let handshake = ControlMessage::Handshake(version, handshake);
                    // let message = Message::Binary(Bytes::from(handshake.serialize()));
                    // let res = socket_write.send(message);
                    // if let Err(e) = res {
                    //     println!("Error for sending hadnskae: {:?}", e); // TODO: log error
                    //     return rx;
                    // }

                    loop {
                        // // wait for the message from the channel
                        // let message = rx.recv().unwrap();

                        // // check if the message is terminate
                        // if message.is_none() {
                        //     socket_write.flush().unwrap();
                        //     break;
                        // }
                        // let message = message.unwrap();
                        // let data = match message {
                        //     MessageData::Stack(data, len) => Bytes::copy_from_slice(&data[0..len]),
                        //     MessageData::Heap(data) => Bytes::from(data),
                        // };

                        // // write the message
                        // let res = socket_write.send(Message::Binary(data));
                        // if let Err(e) = res {
                        //     println!("Error for sending message: {:?}", e); // TODO: log error
                        //     break;
                        // }
                    }
                    rx
                })
                .unwrap();

            ui_state.set_state(ConnectionState::Connected);

            // wait for the read thread to finish
            recv_tread.join().unwrap();

            // terminate the send thread
            channel.send(None).unwrap();
            rx = send_thread.join().unwrap();

            ui_state.set_state(ConnectionState::Disconnected);
        }
    });
}

pub struct ClientBuilder {
    creator: ValuesCreator,
    channel: Sender<Option<MessageData>>,
    rx: Receiver<Option<MessageData>>,
}

impl ClientBuilder {
    pub fn new() -> Self {
        let (channel, rx) = std::sync::mpsc::channel();

        let creator = ValuesCreator::new(ClientChannel::new(channel.clone()));

        Self {
            creator,
            channel,
            rx,
        }
    }

    pub fn creator(&mut self) -> &mut ValuesCreator {
        &mut self.creator
    }

    pub fn build(self, context: Context, addr: Ipv4Addr, port: u16, handshake: u64) -> UIState {
        let Self {
            creator,
            channel,
            rx,
        } = self;

        let addr = SocketAddrV4::new(addr, port);
        let (values, version) = creator.get_values();
        let ui_state = UIState::new(context, ClientChannel::new(channel.clone()));
        start_gui_client(
            addr,
            values,
            version,
            rx,
            channel,
            ui_state.clone(),
            handshake,
        );

        ui_state
    }
}
