use std::net::{Ipv4Addr, SocketAddrV4};
use std::thread;

use egui::Context;
use futures_util::{SinkExt, StreamExt};
use tokio::runtime::Builder;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::{Bytes, Message, protocol::WebSocketConfig};

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
    ui_state: Client,
    handshake: u64,
) {
    loop {
        // wait for the connection signal
        ui_state.wait_connection().await;
        ui_state.set_state(ConnectionState::NotConnected);

        // try to connect to the server
        let address = format!("ws://{}/ws", addr);
        let mut websocket_config = WebSocketConfig::default();
        websocket_config.max_message_size = Some(536870912); // 512 MB
        websocket_config.max_frame_size = Some(536870912); // 512 MB
        let res = connect_async_with_config(&address, Some(websocket_config), false).await;
        if res.is_err() {
            #[cfg(debug_assertions)]
            println!("connecting to server at {:?} failed: {:?}", address, res.err());
            continue;
        }

        // get the socket
        let socket = res.unwrap().0;

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

        let recv_future = tokio::spawn(async move {
            loop {
                // read the message
                let res = socket_read.next().await;
                if res.is_none() {
                    #[cfg(debug_assertions)]
                    println!("Connection closed by server");
                    break;
                }
                let res = res.unwrap();

                if let Err(e) = res {
                    #[cfg(debug_assertions)]
                    println!("reading message from server failed: {:?}", e);
                    break;
                }
                let message = res.unwrap();
                let mess = match message {
                    Message::Binary(d) => d,
                    Message::Close(_) => break,
                    _ => {
                        let error =
                            format!("client received unexpected message type: {:?}", message);
                        th_sender.send(ControlMessage::error(error));
                        break;
                    }
                };

                // handle the message
                let res = handle_message(mess.as_ref(), &th_vals, &th_ui_state);
                if let Err(e) = res {
                    let error = format!("handling message from server failed: {:?}", e);
                    th_sender.send(ControlMessage::error(error));
                    break;
                }
            }
        });

        // send -----------------------------------------
        let send_future = tokio::spawn(async move {
            let handshake = ControlMessage::Handshake(version, handshake);
            let message = Message::Binary(Bytes::from(handshake.serialize()));
            let res = socket_write.send(message).await;
            if let Err(e) = res {
                #[cfg(debug_assertions)]
                println!("sending handshake failed: {:?}", e);
                return rx;
            }

            loop {
                // wait for the message from the channel
                let message = rx.recv().await.unwrap();

                // check if the message is terminate
                if message.is_none() {
                    let _ = socket_write.flush().await;
                    break;
                }
                let message = message.unwrap();
                let data = match message {
                    MessageData::Stack(data, len) => Bytes::copy_from_slice(&data[0..len]),
                    MessageData::Heap(data) => Bytes::from(data),
                };

                // write the message
                let res = socket_write.send(Message::Binary(data)).await;
                if let Err(e) = res {
                    #[cfg(debug_assertions)]
                    println!("sending message to socket failed: {:?}", e);
                    break;
                }
            }
            rx
        });

        ui_state.set_state(ConnectionState::Connected);

        // wait for the read thread to finish
        let _ = recv_future.await;

        // terminate the send thread
        sender.close();
        rx = send_future.await.unwrap();

        ui_state.set_state(ConnectionState::Disconnected);
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

    pub fn addr(self, addr: Ipv4Addr) -> Self {
        Self { addr, ..self }
    }

    pub fn context(self, context: Context) -> Self {
        Self {
            context: Some(context),
            ..self
        }
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

        let runtime = Builder::new_current_thread()
            .thread_name("Client Runtime")
            .enable_io()
            .worker_threads(2)
            .build()
            .unwrap();

        let client_out = client.clone();
        let thread = thread::Builder::new().name("Client".to_string());

        let _ = thread.spawn(move || {
            runtime.block_on(start_gui_client(
                addr, values, version, rx, sender, client, handshake,
            ))
        });

        (states, client_out)
    }
}
