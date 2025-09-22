use std::net::{Ipv4Addr, SocketAddrV4};

use egui::Context;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::UnboundedReceiver;
use ws_stream_wasm::{WsMessage, WsMeta};

use egui_states_core::controls::ControlMessage;
use egui_states_core::serialization::MessageData;

use crate::State;
use crate::client_base::{Client, ConnectionState};
use crate::handle_message::handle_message;
use crate::sender::MessageSender;
use crate::values_creator::{ClientValuesCreator, ValuesList};

async fn start_gui_client(
    addr: SocketAddrV4,
    vals: ValuesList,
    version: u64,
    mut rx: UnboundedReceiver<Option<MessageData>>,
    sender: MessageSender,
    client: Client,
    handshake: u64,
) {
    loop {
        // wait for the connection signal
        client.wait_connection().await;
        client.set_state(ConnectionState::NotConnected);

        // try to connect to the server
        let address = format!("ws://{}/ws", addr);
        let res = WsMeta::connect(&address, None).await;
        if res.is_err() {
            log::error!("Error connecting to server at {}: {:?}", address, res.err());
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
        let th_client = client.clone();
        let th_sender = sender.clone();

        let recv_future = async move {
            loop {
                // read the message
                #[cfg(debug_assertions)]
                log::debug!("Waiting for message...");
                let res = socket_read.next().await;
                if res.is_none() {
                    log::error!("Error reading message: Connection closed by server");
                    break;
                }
                let message = res.unwrap();
                let mess = match message {
                    WsMessage::Binary(d) => d,
                    _ => {
                        log::error!("Wrong type of message received: {:?}", message);
                        break;
                    }
                };

                #[cfg(debug_assertions)]
                log::debug!("Message received: {} bytes", mess.len());

                // handle the message
                let res = handle_message(&mess, &th_vals, &th_client);
                if let Err(e) = res {
                    let error = format!("Error handling message: {:?}", e);
                    log::error!("Error handling message: {}", error);
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
                log::error!("Error for sending handshake: {:?}", e);
                return rx;
            }

            loop {
                // wait for the message from the channel
                let message = rx.recv().await.unwrap();

                // check if the message is terminate
                if message.is_none() {
                    socket_write.flush().await.unwrap();
                    #[cfg(debug_assertions)]
                    log::debug!("Connection closed by client");
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
                    log::error!("Error for sending message: {:?}", e);
                    break;
                }
            }
            rx
        };

        client.set_state(ConnectionState::Connected);

        let (_, rx_) = tokio::join!(recv_future, send_future);
        rx = rx_;

        client.set_state(ConnectionState::Disconnected);
    }
}

pub struct ClientBuilder {
    creator: ClientValuesCreator,
    sender: MessageSender,
    rx: UnboundedReceiver<Option<MessageData>>,
    addr: Ipv4Addr,
    context: Option<Context>,
}

impl ClientBuilder {
    pub fn new() -> Self {
        let (sender, rx) = MessageSender::new();

        let creator = ClientValuesCreator::new(sender.clone());
        let addr = Ipv4Addr::new(127, 0, 0, 1);

        Self {
            creator,
            sender,
            rx,
            addr,
            context: None,
        }
    }

    pub fn context(self, context: Context) -> Self {
        Self {
            context: Some(context),
            ..self
        }
    }

    pub fn addr(self, addr: Ipv4Addr) -> Self {
        Self { addr, ..self }
    }

    pub fn build<T: State>(self, port: u16, handshake: u64) -> (T, Client) {
        let Self {
            mut creator,
            sender,
            rx,
            addr,
            context,
        } = self;

        let addr = SocketAddrV4::new(addr, port);
        let states = T::new(&mut creator);
        let (values, version) = creator.get_values();
        let client = Client::new(context, sender.clone());

        let client_out = client.clone();

        wasm_bindgen_futures::spawn_local(async move {
            start_gui_client(addr, values, version, rx, sender, client, handshake).await;
        });

        (states, client_out)
    }
}
