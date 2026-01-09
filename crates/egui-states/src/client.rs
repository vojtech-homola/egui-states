use std::net::{Ipv4Addr, SocketAddrV4};

use egui::Context;
use tokio::sync::mpsc::UnboundedReceiver;

use egui_states_core::PROTOCOL_VERSION;
use egui_states_core::serialization::{ClientHeader, MessageData, to_message_data};

use crate::State;
use crate::client_base::{Client, ConnectionState};
use crate::client_states::{StatesCreatorClient, ValuesList};
use crate::handle_message::{handle_message, parse_to_send};
use crate::sender::{ChannelMessage, MessageSender};

#[cfg(not(target_arch = "wasm32"))]
use crate::websocket::build_ws;

#[cfg(target_arch = "wasm32")]
use crate::websocket_wasm::build_ws;

async fn start_gui_client(
    addr: SocketAddrV4,
    vals: ValuesList,
    mut rx: UnboundedReceiver<Option<ChannelMessage>>,
    sender: MessageSender,
    ui_state: Client,
    handshake: u64,
) {
    loop {
        // wait for the connection signal
        ui_state.wait_connection().await;
        ui_state.set_state(ConnectionState::NotConnected);

        // try to connect to the server
        let res = build_ws(addr).await;
        if res.is_err() {
            continue;
        }
        let (mut socket_read, mut socket_send) = res.unwrap();

        // clean message queue before starting
        while !rx.is_empty() {
            let _ = rx.recv().await;
        }

        // communicate handshake and initialization -------------------------
        let message =
            ClientHeader::serialize_handshake(PROTOCOL_VERSION, handshake, vals.types.clone());
        if let Err(_) = socket_send.send(message).await {
            #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
            println!("Sending handshake failed.");
            #[cfg(all(debug_assertions, target_arch = "wasm32"))]
            log::error!("Sending handshake failed.");
            continue;
        }

        // read -----------------------------------------
        let th_vals = vals.clone();
        let th_ui_state = ui_state.clone();
        let th_sender = sender.clone();

        let recv_future = async move {
            loop {
                // read the message
                match socket_read.read().await {
                    Ok(data) => {
                        if let Err(e) = handle_message(data.as_ref(), &th_vals, &th_ui_state).await
                        {
                            let error = format!("handling message from server failed: {:?}", e);
                            th_sender.send(ChannelMessage::Error(error));
                            // break; TODO: decide if we want to break the loop on error
                        }
                    }
                    Err(_) => break,
                }
            }
            th_sender.close();
        };

        #[cfg(not(target_arch = "wasm32"))]
        let recv_future = tokio::spawn(recv_future);

        // send -----------------------------------------
        let send_future = async move {
            loop {
                match rx.recv().await.unwrap() {
                    Some(msg) => {
                        let message = MessageData::new();
                        let mut message = parse_to_send(msg, message);
                        let stop = loop {
                            match rx.try_recv() {
                                Ok(Some(msg)) => {
                                    message = parse_to_send(msg, message);
                                }
                                Ok(None) => {
                                    let _ = socket_send.send(message).await;
                                    break true;
                                }
                                Err(_) => {
                                    if let Err(_) = socket_send.send(message).await {
                                        break true;
                                    }
                                    break false;
                                }
                            }
                        };

                        if stop {
                            break;
                        }

                        // // let message = to_message_data(&header, data);
                        // // write the message
                        // if let Err(_) = socket_send.send(message).await {
                        //     break;
                        // }
                    }
                    // check if the message is terminate
                    None => {
                        break;
                    }
                }
            }
            socket_send.close().await;
            rx
        };

        #[cfg(not(target_arch = "wasm32"))]
        let send_future = tokio::spawn(send_future);

        ui_state.set_state(ConnectionState::Connected);

        #[cfg(not(target_arch = "wasm32"))]
        {
            // wait for the read thread to finish
            let _ = recv_future.await;

            // wait for the send thread
            rx = send_future.await.unwrap();
        }

        #[cfg(target_arch = "wasm32")]
        {
            let (_, rx_) = tokio::join!(recv_future, send_future);
            rx = rx_;
        }

        ui_state.set_state(ConnectionState::Disconnected);
    }
}

pub struct ClientBuilder {
    creator: StatesCreatorClient,
    sender: MessageSender,
    rx: UnboundedReceiver<Option<ChannelMessage>>,
    addr: Ipv4Addr,
    context: Option<Context>,
}

impl ClientBuilder {
    pub fn new() -> Self {
        let (sender, rx) = MessageSender::new();

        let creator = StatesCreatorClient::new(sender.clone(), "root".to_string());
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
        let values = creator.get_values();
        let client = Client::new(context, sender.clone());
        let client_out = client.clone();

        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::thread;
            use tokio::runtime::Builder;

            let runtime = Builder::new_current_thread()
                .thread_name("Client Runtime")
                .enable_io()
                .worker_threads(2)
                .build()
                .unwrap();

            let thread = thread::Builder::new().name("Client".to_string());

            let _ = thread.spawn(move || {
                runtime.block_on(start_gui_client(
                    addr, values, rx, sender, client, handshake,
                ))
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            wasm_bindgen_futures::spawn_local(async move {
                start_gui_client(addr, values, rx, sender, client, handshake).await;
            });
        }

        (states, client_out)
    }
}
