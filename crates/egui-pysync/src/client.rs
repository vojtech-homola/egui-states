use std::io::Write;
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use egui::Context;

use crate::client_state::{ConnectionState, UIState};
use crate::commands::CommandMessage;
use crate::states_creator::{ValuesCreator, ValuesList};
use crate::transport::{read_message, write_message, MessageData, ReadMessage, WriteMessage};

fn handle_message(
    message: ReadMessage,
    vals: &ValuesList,
    ui_state: &UIState,
) -> Result<(), String> {
    if let ReadMessage::Command(ref command) = message {
        match command {
            CommandMessage::Update(t) => {
                ui_state.update(*t);
            }
            _ => {}
        }
        return Ok(());
    }

    let update = match message {
        ReadMessage::Value(id, updata, data) => match vals.values.get(&id) {
            Some(value) => {
                match data {
                    MessageData::Stack(data) => value.update_value(&data),
                    MessageData::Heap(data) => value.update_value(&data),
                }?;
                updata
            }
            None => return Err(format!("Value with id {} not found", id)),
        },

        ReadMessage::Static(id, updata, data) => match vals.static_values.get(&id) {
            Some(value) => {
                match data {
                    MessageData::Stack(data) => value.update_value(&data),
                    MessageData::Heap(data) => value.update_value(&data),
                }?;
                updata
            }
            None => return Err(format!("Static with id {} not found", id)),
        },

        ReadMessage::Image(id, updata, data) => match vals.images.get(&id) {
            Some(value) => {
                match data {
                    MessageData::Stack(data) => value.update_image(&data),
                    MessageData::Heap(data) => value.update_image(&data),
                }?;
                updata
            }
            None => return Err(format!("Image with id {} not found", id)),
        },

        ReadMessage::Dict(id, updata, data) => match vals.dicts.get(&id) {
            Some(value) => {
                value.update_dict(data)?;
                updata
            }
            None => return Err(format!("Dict with id {} not found", id)),
        },

        ReadMessage::List(id, updata, data) => match vals.lists.get(&id) {
            Some(value) => {
                value.update_list(data)?;
                updata
            }
            None => return Err(format!("List with id {} not found", id)),
        },

        ReadMessage::Graph(id, updata, data) => match vals.graphs.get(&id) {
            Some(value) => {
                match data {
                    MessageData::Stack(data) => value.update_graph(&data),
                    MessageData::Heap(data) => value.update_graph(&data),
                }?;
                updata
            }
            None => return Err(format!("Graph with id {} not found", id)),
        },

        ReadMessage::Signal(_, _) => {
            return Err("Signal message should not be handled in the client".to_string());
        }

        ReadMessage::Command(_) => unreachable!("should not parse Command message"),
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
    mut rx: Receiver<WriteMessage>,
    channel: Sender<WriteMessage>,
    ui_state: UIState,
    handshake: u64,
) {
    let client_thread = thread::Builder::new().name("Client".to_string());
    let _ = client_thread.spawn(move || loop {
        // wait for the connection signal
        ui_state.wait_connection();
        ui_state.set_state(ConnectionState::NotConnected);

        // try to connect to the server
        let res = TcpStream::connect(addr);
        if res.is_err() {
            continue;
        }

        // get the stream
        let mut stream_write = res.unwrap();
        let mut stream_read = stream_write.try_clone().unwrap();

        // clean mesage queue before starting
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
                    let res = read_message(&mut stream_read);
                    if let Err(e) = res {
                        println!("Error reading message: {:?}", e); // TODO: log error
                        break;
                    }
                    let message = res.unwrap();

                    // handle the message
                    let res = handle_message(message, &th_vals, &th_ui_state);
                    if let Err(e) = res {
                        let error = format!("Error handling message: {:?}", e);
                        th_channel
                            .send(WriteMessage::Command(CommandMessage::Error(error)))
                            .unwrap();
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
                let handshake = CommandMessage::Handshake(version, handshake);
                let message = WriteMessage::Command(handshake);
                let res = write_message(message, &mut stream_write);
                if let Err(e) = res {
                    println!("Error for sending hadnskae: {:?}", e); // TODO: log error
                    return rx;
                }

                loop {
                    // wait for the message from the channel
                    let message = rx.recv().unwrap();

                    // check if the message is terminate
                    if let WriteMessage::Terminate = message {
                        stream_write.flush().unwrap();
                        break;
                    }

                    // write the message
                    let res = write_message(message, &mut stream_write);
                    if let Err(e) = res {
                        println!("Error for sending message: {:?}", e); // TODO: log error
                        break;
                    }
                }
                rx
            })
            .unwrap();

        ui_state.set_state(ConnectionState::Connected);

        // wait for the read thread to finish
        recv_tread.join().unwrap();

        // terminate the send thread
        channel.send(WriteMessage::Terminate).unwrap();
        rx = send_thread.join().unwrap();

        ui_state.set_state(ConnectionState::Disconnected);
    });
}

pub struct ClientBuilder {
    creator: ValuesCreator,
    channel: Sender<WriteMessage>,
    rx: Receiver<WriteMessage>,
}

impl ClientBuilder {
    pub fn new() -> Self {
        let (channel, rx) = std::sync::mpsc::channel();
        let creator = ValuesCreator::new(channel.clone());

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
        let ui_state = UIState::new(context, channel.clone());
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
