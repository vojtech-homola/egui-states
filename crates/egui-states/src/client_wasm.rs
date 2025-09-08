use std::net::{Ipv4Addr, SocketAddrV4};

use egui::Context;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::UnboundedReceiver;
use ws_stream_wasm::{WsMessage, WsMeta};

use egui_states_core::controls::ControlMessage;
use egui_states_core::serialization::MessageData;

use crate::State;
use crate::client_state::{ConnectionState, UIState};
use crate::handle_message::handle_message;
use crate::sender::MessageSender;
use crate::states_creator::{ValuesCreator, ValuesList};

async fn start_gui_client(
    addr: SocketAddrV4,
    vals: ValuesList,
    version: u64,
    mut rx: UnboundedReceiver<Option<MessageData>>,
    sender: MessageSender,
    ui_state: UIState,
    handshake: u64,
) {
    loop {
        // wait for the connection signal
        ui_state.wait_connection().await;
        ui_state.set_state(ConnectionState::NotConnected);

        // try to connect to the server
        let address = format!("ws://{}", addr);
        let res = WsMeta::connect(&address, None).await;
        if res.is_err() {
            continue;
        }

        // get the socket
        let socket = res.unwrap().1;

        // clean message queue before starting
        while !rx.is_empty() {
            let _ = rx.recv().await;
        }

        // split the socket
        let (mut socket_write, mut socket_read) = socket.split();

        // read -----------------------------------------
        let th_vals = vals.clone();
        let th_ui_state = ui_state.clone();
        let th_sender = sender.clone();

        let recv_future = async move {
            loop {
                // read the message
                let res = socket_read.next().await;
                if res.is_none() {
                    // println!("Error reading message: {:?}", e); // TODO: log error
                    println!("Error reading message: Connection closed by server");
                    break;
                }
                let message = res.unwrap();
                let mess = match message {
                    WsMessage::Binary(d) => d,
                    _ => {
                        println!("Wrong type of message received: {:?}", message); // TODO: log error
                        break;
                    }
                };

                // handle the message
                let res = handle_message(&mess, &th_vals, &th_ui_state);
                if let Err(e) = res {
                    let error = format!("Error handling message: {:?}", e);
                    th_sender.send(ControlMessage::error(error));
                    break;
                }
            }
            th_sender.close();
        };

        // send -----------------------------------------
        let send_future = async move {
            let handshake = ControlMessage::Handshake(version, handshake);
            let message = WsMessage::Binary(handshake.serialize());
            let res = socket_write.send(message).await;
            if let Err(e) = res {
                println!("Error for sending handshake: {:?}", e); // TODO: log error
                return rx;
            }

            loop {
                // wait for the message from the channel
                let message = rx.recv().await.unwrap();

                // check if the message is terminate
                if message.is_none() {
                    socket_write.flush().await.unwrap();
                    break;
                }
                let message = message.unwrap();
                let message = match message {
                    MessageData::Stack(data, len) => WsMessage::Binary((&data[0..len]).to_vec()),
                    MessageData::Heap(data) => WsMessage::Binary(data),
                };

                // write the message
                let res = socket_write.send(message).await;
                if let Err(e) = res {
                    println!("Error for sending message: {:?}", e); // TODO: log error
                    break;
                }
            }
            rx
        };

        ui_state.set_state(ConnectionState::Connected);

        let (_, rx_) = tokio::join!(recv_future, send_future);
        rx = rx_;

        ui_state.set_state(ConnectionState::Disconnected);
    }
}

pub struct ClientBuilder {
    creator: ValuesCreator,
    sender: MessageSender,
    rx: UnboundedReceiver<Option<MessageData>>,
    addr: Ipv4Addr,
}

impl ClientBuilder {
    pub fn new() -> Self {
        let (sender, rx) = MessageSender::new();

        let creator = ValuesCreator::new(sender.clone());
        let addr = Ipv4Addr::new(127, 0, 0, 1);

        Self {
            creator,
            sender,
            rx,
            addr,
        }
    }

    pub fn addr(self, addr: Ipv4Addr) -> Self {
        Self { addr, ..self }
    }

    pub fn build<T: State>(self, context: Context, port: u16, handshake: u64) -> (T, UIState) {
        let Self {
            mut creator,
            sender,
            rx,
            addr,
        } = self;

        let addr = SocketAddrV4::new(addr, port);
        let states = T::new(&mut creator);
        let (values, version) = creator.get_values();
        let ui_state = UIState::new(context, sender.clone());

        let ui_state_cl = ui_state.clone();

        wasm_bindgen_futures::spawn_local(async move {
            start_gui_client(addr, values, version, rx, sender, ui_state_cl, handshake).await;
        });

        (states, ui_state)
    }
}
