use std::net::SocketAddrV4;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, atomic};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

use egui_states_core::PROTOCOL_VERSION;
use egui_states_core::controls::{ControlClient, ControlServer};
use egui_states_core::event_async::Event;
use egui_states_core::serialization::{
    ClientHeader, ServerHeader, deserialize, deserialize_from, serialize_value_vec,
};

use crate::sender::{MessageReceiver, MessageSender};
use crate::server::ServerStatesList;
use crate::signals::ChangedValues;

enum ChannelHolder {
    Transfer(JoinHandle<MessageReceiver>),
    Rx(MessageReceiver),
}

pub(crate) async fn start(
    sender: MessageSender,
    rx: MessageReceiver,
    connected: Arc<atomic::AtomicBool>,
    enabled: Arc<atomic::AtomicBool>,
    values: ServerStatesList,
    signals: ChangedValues,
    start_event: Event,
    addr: SocketAddrV4,
    handshake: Option<Vec<u64>>,
) {
    let mut holder = ChannelHolder::Rx(rx);

    loop {
        // wait for start control event
        start_event.wait().await;

        // listen to incoming connections
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                signals.error(&format!("binding failed: {:?}", e));
                continue;
            }
        };

        // accept incoming connection
        let stream = listener.accept().await;

        // if server is disabled, go back and wait for start control event
        if !enabled.load(atomic::Ordering::Relaxed) {
            if let Ok((mut stream, _)) = stream {
                let _ = stream.shutdown().await;
            }
            continue;
        }

        // check if error accepting connection
        let stream = match stream {
            Ok(s) => s.0,
            Err(e) => {
                signals.error(&format!("accepting connection failed: {:?}", e));
                continue;
            }
        };

        let mut websocket_config = WebSocketConfig::default();
        websocket_config.max_message_size = Some(536870912); // 512 MB
        websocket_config.max_frame_size = Some(536870912); // 512 MB
        let mut websocket =
            match tokio_tungstenite::accept_async_with_config(stream, Some(websocket_config)).await
            {
                Ok(ws) => ws,
                Err(e) => {
                    signals.error(&format!("websocket handshake failed: {:?}", e));
                    connected.store(false, atomic::Ordering::Relaxed);
                    continue;
                }
            };

        // read the message
        let message = match websocket.next().await {
            Some(Ok(message)) => message,
            Some(Err(e)) => {
                signals.error(&format!("reading initial message failed: {:?}", e));
                connected.store(false, atomic::Ordering::Relaxed);
                continue;
            }
            None => {
                signals.error("reading initial message failed");
                connected.store(false, atomic::Ordering::Relaxed);
                continue;
            }
        };

        if let Message::Binary(message) = message {
            let header = match ClientHeader::deserialize_header(&message) {
                Ok((h, _)) => h,
                Err(_) => {
                    signals.error("deserializing initial message header failed");
                    continue;
                }
            };

            if let ClientHeader::Control(ControlClient::Handshake(p, h)) = header {
                if p != PROTOCOL_VERSION {
                    let message = format!(
                        "attempted to connect with wrong protocol version: expected {}, got {}",
                        PROTOCOL_VERSION, p
                    );
                    signals.warning(&message);
                    continue;
                }

                if let Some(ref hash) = handshake {
                    if !hash.contains(&h) {
                        signals.warning("attempted to connect with wrong hash");
                        continue;
                    }
                }

                // check types --------------------------
                let header = ServerHeader::Control(ControlServer::TypesAsk);
                let mut data = Vec::new();
                serialize_value_vec(&header, &mut data);
                serialize_value_vec(&values.types, &mut data);
                let message = Bytes::from_owner(data);

                if let Err(e) = websocket.send(Message::Binary(message)).await {
                    signals.error(&format!("sending states types failed: {:?}", e));
                    continue;
                }

                // TODO: move to special function
                let types = match websocket.next().await {
                    Some(Ok(Message::Binary(data))) => {
                        match deserialize_from::<ClientHeader>(&data) {
                            Ok((ClientHeader::Control(ControlClient::TypesAnswer), dat)) => {
                                match deserialize::<Vec<u64>>(&dat) {
                                    Ok(types) => types,
                                    Err(_) => {
                                        signals.error(
                                        "unexpected message when receiving initial states types",
                                    );
                                        continue;
                                    }
                                }
                            }
                            _ => {
                                signals.error(
                                    "unexpected message when receiving initial states types",
                                );
                                continue;
                            }
                        }
                    }
                    _ => {
                        signals.error("receiving initial states types failed");
                        continue;
                    }
                };
                // --------------------------------------

                let mut rx = match holder {
                    // disconnect previous client
                    ChannelHolder::Transfer(handler) => {
                        #[cfg(debug_assertions)]
                        signals.debug("terminating previous connection");
                        connected.store(false, atomic::Ordering::Relaxed);
                        for (_, v) in &values.enable {
                            v.enable(false);
                        }
                        sender.close();
                        let rx = handler.await.expect("joining communication handler failed");
                        rx
                    }
                    ChannelHolder::Rx(rx) => rx,
                };

                // clean mesage queue and send sync signals
                while !rx.is_empty() {
                    let _ = rx.recv().await;
                }

                for id in &types {
                    if let Some(v) = values.enable.get(id) {
                        v.enable(true);
                    }
                }

                // std::thread::sleep(std::time::Duration::from_millis(100));
                connected.store(true, atomic::Ordering::Relaxed);
                for v in values.sync.iter() {
                    v.sync();
                }

                // start transfer thread
                let handler = communication_handler(
                    connected.clone(),
                    values.clone(),
                    signals.clone(),
                    websocket,
                    rx,
                    sender.clone(),
                )
                .await;
                holder = ChannelHolder::Transfer(handler);
            }
        }
    }
}

async fn communication_handler(
    connected: Arc<AtomicBool>,
    values: ServerStatesList,
    signals: ChangedValues,
    websocket: WebSocketStream<TcpStream>,
    rx: MessageReceiver,
    sender: MessageSender,
) -> JoinHandle<MessageReceiver> {
    let (socket_tx, mut socket_rx) = websocket.split();

    let read_connected = connected.clone();
    let read_signals = signals.clone();
    let read_values = values.clone();
    let read_sender = sender.clone();

    let reader_handler = tokio::spawn(async move {
        loop {
            // read the message
            let result_message = socket_rx.next().await;

            // check if not connected
            if !read_connected.load(atomic::Ordering::Relaxed) {
                #[cfg(debug_assertions)]
                read_signals.debug("read thread is closing");
                read_signals.reset();
                break;
            }

            let message = match result_message {
                Some(Ok(m)) => m,
                Some(Err(e)) => {
                    read_signals.error(&format!("reading message from client failed: {:?}", e));
                    read_connected.store(false, atomic::Ordering::Relaxed);
                    read_signals.reset();
                    break;
                }
                None => {
                    read_signals.info("connection was closed by the client");
                    read_connected.store(false, atomic::Ordering::Relaxed);
                    read_signals.reset();
                    break;
                }
            };

            match message {
                Message::Binary(m) => {
                    let (header, data) = match ClientHeader::deserialize_header(&m) {
                        Ok(hd) => hd,
                        Err(_) => {
                            read_signals.error(&format!("deserializing message header failed"));
                            continue;
                        }
                    };

                    match header {
                        ClientHeader::Control(control) => match control {
                            ControlClient::Ack(v) => {
                                let val_res = read_values.ack.get(&v);
                                match val_res {
                                    Some(val) => {
                                        val.acknowledge();
                                    }
                                    None => read_signals.error(&format!(
                                        "value with id {} not found for Acknowledge",
                                        v
                                    )),
                                }
                            }
                            ControlClient::Error => {
                                let err = match deserialize::<String>(&data.unwrap()) {
                                    Ok(e) => e,
                                    Err(_) => {
                                        read_signals.error(
                                            "deserializing error message from client failed",
                                        );
                                        continue;
                                    }
                                };
                                read_signals.error(&format!("Error message from client: {}", err));
                            }
                            _ => read_signals.error(&format!(
                                "Command {} should not be processed here",
                                control.as_str()
                            )),
                        },
                        ClientHeader::Value(id, signal) => match read_values.values.get(&id) {
                            Some(val) => {
                                if let Err(e) = val.update_value(signal, data.unwrap()) {
                                    read_signals.error(&format!(
                                        "updating value with id {} failed: {}",
                                        id, e
                                    ));
                                }
                            }
                            None => read_signals.error(&format!("value with id {} not found", id)),
                        },
                        ClientHeader::Signal(id) => match read_values.signals.get(&id) {
                            Some(val) => {
                                if let Err(e) = val.update_signal(data.unwrap()) {
                                    read_signals.error(&format!(
                                        "updating signal with id {} failed: {}",
                                        id, e
                                    ));
                                }
                            }
                            None => read_signals.error(&format!("value with id {} not found", id)),
                        },
                    }
                }
                _ => read_signals.error("Unexpected message format"),
            };
        }

        // acknowledge all pending values
        for v in values.ack.values() {
            v.acknowledge();
        }

        // send close signal to writing thread if reading fails
        #[cfg(debug_assertions)]
        read_signals.debug("terminating write thread");
        read_sender.close();
    });

    tokio::spawn(writer(rx, connected, socket_tx, signals, reader_handler))
}

async fn writer(
    mut rx: MessageReceiver,
    connected: Arc<AtomicBool>,
    mut websocket: SplitSink<WebSocketStream<TcpStream>, Message>,
    signals: ChangedValues,
    reader_handle: tokio::task::JoinHandle<()>,
) -> MessageReceiver {
    loop {
        // get message from channel
        let msg = match rx
            .recv()
            .await
            .expect("receiving message from channel failed")
        {
            Some(m) => m,
            // check if message is terminate signal
            None => {
                signals.info("writer is closing connection");
                let _ = websocket.close().await;
                reader_handle.abort();
                let _ = reader_handle.await;
                break;
            }
        };

        // if not connected, stop thread
        if !connected.load(atomic::Ordering::Relaxed) {
            let _ = websocket.close().await;
            reader_handle.abort();
            let _ = reader_handle.await;
            break;
        }

        // send message
        if let Err(e) = websocket.send(msg).await {
            signals.error(&format!("sending message to client failed: {:?}", e));
            reader_handle.abort();
            let _ = reader_handle.await;
            break;
        }
    }
    rx
}
