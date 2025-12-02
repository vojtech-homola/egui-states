use std::net::{Ipv4Addr, SocketAddrV4};

use egui::Context;
use tokio::sync::mpsc::UnboundedReceiver;

use egui_states_core::PROTOCOL_VERSION;
use egui_states_core::serialization::ClientHeader;

use crate::State;
use crate::client_base::{Client, ConnectionState};
use crate::handle_message::{check_types, handle_message};
use crate::sender::{ChannelMessage, MessageSender};
use crate::values_creator::{ClientValuesCreator, ValuesList};

#[cfg(feature = "client")]
use crate::websocket::build_ws;

#[cfg(feature = "client-wasm")]
use crate::websocket_wasm::build_ws;

async fn start_gui_client(
    addr: SocketAddrV4,
    vals: ValuesList,
    mut rx: UnboundedReceiver<ChannelMessage>,
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
        let message = ClientHeader::serialize_handshake(PROTOCOL_VERSION, handshake);
        if let Err(_) = socket_send.send(message).await {
            #[cfg(debug_assertions)]
            println!("Sending handshake failed.");
            continue;
        }

        // process and send states types
        match socket_read.read().await {
            Ok(data) => match check_types(data.as_ref(), &vals) {
                Ok(message) => {
                    if let Err(_) = socket_send.send(message).await {
                        #[cfg(debug_assertions)]
                        println!("Sending states types failed.");
                        continue;
                    }
                }
                Err(_) => continue,
            },
            Err(_) => {
                #[cfg(debug_assertions)]
                println!("Receiving states types failed.");
                continue;
            }
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
                            let (header, data) = ClientHeader::error(error);
                            th_sender.send_data(header, data);
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        };

        #[cfg(feature = "client")]
        let recv_future = tokio::spawn(recv_future);

        // send -----------------------------------------
        let send_future = async move {
            loop {
                // wait for the message from the channel
                match rx.recv().await.unwrap() {
                    Some((header, data)) => {
                        let message = header.serialize_message(data);
                        // write the message
                        if let Err(_) = socket_send.send(message).await {
                            break;
                        }
                    }
                    // check if the message is terminate
                    None => {
                        socket_send.flush().await;
                        break;
                    }
                }
            }
            rx
        };

        #[cfg(feature = "client")]
        let send_future = tokio::spawn(send_future);

        ui_state.set_state(ConnectionState::Connected);

        #[cfg(feature = "client")]
        {
            // wait for the read thread to finish
            let _ = recv_future.await;

            // terminate the send thread
            sender.close();
            rx = send_future.await.unwrap();
        }

        #[cfg(feature = "client-wasm")]
        {
            let (_, rx_) = tokio::join!(recv_future, send_future);
            rx = rx_;
        }

        ui_state.set_state(ConnectionState::Disconnected);
    }
}

pub struct ClientBuilder {
    creator: ClientValuesCreator,
    sender: MessageSender,
    rx: UnboundedReceiver<ChannelMessage>,
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
        let values = creator.get_values();
        let client = Client::new(context, sender.clone());
        let client_out = client.clone();

        #[cfg(feature = "client")]
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

        #[cfg(feature = "client-wasm")]
        {
            wasm_bindgen_futures::spawn_local(async move {
                start_gui_client(addr, values, rx, sender, client, handshake).await;
            });
        }

        (states, client_out)
    }
}
