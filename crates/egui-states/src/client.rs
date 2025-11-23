use std::net::{Ipv4Addr, SocketAddrV4};
use std::thread;

use egui::Context;
use tokio::runtime::Builder;
use tokio::sync::mpsc::UnboundedReceiver;

use egui_states_core::serialization::ClientHeader;

use crate::State;
use crate::client_base::{Client, ConnectionState};
use crate::handle_message::handle_message;
use crate::sender::{ChannelMessage, MessageSender};
use crate::values_creator::{ClientValuesCreator, ValuesList};

#[cfg(feature = "client")]
use crate::websocket::build_ws;

async fn start_gui_client(
    addr: SocketAddrV4,
    vals: ValuesList,
    version: u64,
    mut rx: UnboundedReceiver<ChannelMessage>,
    sender: MessageSender,
    ui_state: Client,
    handshake: u64,
) {
    loop {
        // wait for the connection signal
        ui_state.wait_connection().await;
        ui_state.set_state(ConnectionState::NotConnected);

        // // try to connect to the server
        // let address = format!("ws://{}/ws", addr);
        // let mut websocket_config = WebSocketConfig::default();
        // websocket_config.max_message_size = Some(536870912); // 512 MB
        // websocket_config.max_frame_size = Some(536870912); // 512 MB
        // let res = connect_async_with_config(&address, Some(websocket_config), false).await;
        // if res.is_err() {
        //     #[cfg(debug_assertions)]
        //     println!(
        //         "connecting to server at {:?} failed: {:?}",
        //         address,
        //         res.err()
        //     );
        //     continue;
        // }

        // // get the socket
        // let socket = res.unwrap().0;

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

        // split the socket
        // let (mut socket_write, mut socket_read) = socket.split();

        // read -----------------------------------------
        let th_vals = vals.clone();
        let th_ui_state = ui_state.clone();
        let th_sender = sender.clone();

        let recv_future = tokio::spawn(async move {
            loop {
                // read the message
                // let res = socket_read.read().await;

                match socket_read.read().await {
                    Ok(data) => {
                        if let Err(e) = handle_message(data.as_ref(), &th_vals, &th_ui_state).await
                        {
                            let error = format!("handling message from server failed: {:?}", e);
                            th_sender.send(ClientHeader::error(error));
                            break;
                        }
                    }
                    Err(_) => break,
                }

                // if res.is_none() {
                //     #[cfg(debug_assertions)]
                //     println!("Connection closed by server");
                //     break;
                // }
                // let res = res.unwrap();

                // // #[allow(unused_variables)]
                // if let Err(e) = res {
                //     #[cfg(debug_assertions)]
                //     println!("reading message from server failed: {:?}", e);
                //     break;
                // }
                // let message = res.unwrap();
                // let mess = match message {
                //     Message::Binary(d) => d,
                //     Message::Close(_) => break,
                //     _ => {
                //         let error =
                //             format!("client received unexpected message type: {:?}", message);
                //         th_sender.send(ClientHeader::error(error));
                //         break;
                //     }
                // };

                // // handle the message
                // let res = handle_message(mess.as_ref(), &th_vals, &th_ui_state).await;
                // if let Err(e) = res {
                //     let error = format!("handling message from server failed: {:?}", e);
                //     th_sender.send(ClientHeader::error(error));
                //     break;
                // }
            }
        });

        // send -----------------------------------------
        let send_future = tokio::spawn(async move {
            let message = ClientHeader::serialize_handshake(version, handshake);
            if let Err(_) = socket_send.send(message).await {
                #[cfg(debug_assertions)]
                println!("Sending handshake failed.");
                return rx;
            }

            // #[allow(unused_variables)]
            // if let Err(e) = res {
            //     #[cfg(debug_assertions)]
            //     println!("sending handshake failed: {:?}", e);
            //     return rx;
            // }

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

                // // wait for the message from the channel
                // let message = rx.recv().await.unwrap();

                // // check if the message is terminate
                // if message.is_none() {
                //     let _ = socket_send.flush().await;
                //     break;
                // }
                // let (header, data) = message.unwrap();
                // let data = header.serialize_message(data);

                // // write the message
                // let res = socket_send.send(Message::Binary(data)).await;
                // #[allow(unused_variables)]
                // if let Err(e) = res {
                //     #[cfg(debug_assertions)]
                //     println!("sending message to socket failed: {:?}", e);
                //     break;
                // }
            }
            rx
        });

        ui_state.set_state(ConnectionState::Connected);

        #[cfg(feature = "client")]
        {
            // wait for the read thread to finish
            let _ = recv_future.await;

            // terminate the send thread
            sender.close();
            rx = send_future.await.unwrap();
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
        let (values, version) = creator.get_values();
        let client = Client::new(context, sender.clone());
        let client_out = client.clone();

        #[cfg(feature = "client")]
        {
            let runtime = Builder::new_current_thread()
                .thread_name("Client Runtime")
                .enable_io()
                .worker_threads(2)
                .build()
                .unwrap();

            let thread = thread::Builder::new().name("Client".to_string());

            let _ = thread.spawn(move || {
                runtime.block_on(start_gui_client(
                    addr, values, version, rx, sender, client, handshake,
                ))
            });
        }

        (states, client_out)
    }
}
