use std::net::SocketAddrV4;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, atomic};

use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Bytes, Message};

use egui_states_core::controls::ControlMessage;
use egui_states_core::event::Event;
use egui_states_core::serialization;

use crate::sender::MessageSender;
use crate::signals::ChangedValues;
use crate::states_server::ValuesList;

enum ChannelHolder {
    Transfer(JoinHandle<UnboundedReceiver<Option<Bytes>>>),
    Rx(UnboundedReceiver<Option<Bytes>>),
}

pub(crate) async fn start(
    sender: MessageSender,
    rx: UnboundedReceiver<Option<Bytes>>,
    connected: Arc<atomic::AtomicBool>,
    enabled: Arc<atomic::AtomicBool>,
    values: ValuesList,
    signals: ChangedValues,
    start_event: Event,
    addr: SocketAddrV4,
    version: u64,
    handshake: Option<Vec<u64>>,
) {
    let mut holder = ChannelHolder::Rx(rx);

    loop {
        // wait for start control event
        start_event.wait();

        // listen to incoming connections
        let listener = TcpListener::bind(addr).await;
        if let Err(e) = listener {
            let error = format!("Error binding: {:?}", e);
            signals.set(0, error);
            continue;
        }
        let listener = listener.unwrap();

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
        if let Err(e) = stream {
            let error = format!("Error accepting connection: {:?}", e);
            signals.set(0, error);
            continue;
        }
        let stream = stream.unwrap().0;
        let websocket_res = tokio_tungstenite::accept_async(stream).await;
        if let Err(e) = websocket_res {
            let error = format!("Error during the websocket handshake: {:?}", e);
            signals.set(0, error);
            connected.store(false, atomic::Ordering::Relaxed);
            continue;
        }
        let mut websocket = websocket_res.unwrap();

        // read the message
        let res = websocket.next().await.unwrap();
        if let Err(e) = res {
            let error = format!("Error reading initial message: {:?}", e);
            signals.set(0, error);
            connected.store(false, atomic::Ordering::Relaxed);
            continue;
        }
        let res = res.unwrap();

        if let Message::Binary(message) = res {
            let data = message.as_ref();
            if data[0] == serialization::TYPE_CONTROL {
                let control = ControlMessage::deserialize(data).unwrap(); //TODO handle error
                if let ControlMessage::Handshake(v, h) = control {
                    if v != version {
                        let error = format!(
                            "Attempted to connect with different version: {}, version {} is required.",
                            v, version
                        );
                        signals.set(0, error);
                        continue;
                    }

                    if let Some(ref hash) = handshake {
                        if !hash.contains(&h) {
                            let error = "Attempted to connect with wrong hash".to_string();
                            signals.set(0, error);
                            continue;
                        }
                    }

                    let mut rx = match holder {
                        // disconnect previous client
                        ChannelHolder::Transfer(handler) => {
                            connected.store(false, atomic::Ordering::Relaxed);
                            sender.close();
                            handler.await.unwrap()
                        }
                        ChannelHolder::Rx(rx) => rx,
                    };

                    // clean mesage queue and send sync signals
                    while !rx.is_empty() {
                        rx.recv().await;
                    }

                    connected.store(true, atomic::Ordering::Relaxed);
                    for (_, v) in values.sync.iter() {
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
}

async fn communication_handler(
    connected: Arc<AtomicBool>,
    values: ValuesList,
    signals: ChangedValues,
    websocket: WebSocketStream<TcpStream>,
    rx: UnboundedReceiver<Option<Bytes>>,
    sender: MessageSender,
) -> JoinHandle<UnboundedReceiver<Option<Bytes>>> {
    let connect_w = connected.clone();
    let signals_w = signals.clone();
    let (socket_tx, mut socket_rx) = websocket.split();

    let writer_handle =
        tokio::spawn(async move { writer(rx, connect_w, socket_tx, signals_w).await });

    let handler = tokio::spawn(async move {
        loop {
            // read the message
            let res = socket_rx.next().await;

            // check if not connected
            if !connected.load(atomic::Ordering::Relaxed) {
                break;
            }

            if res.is_none() {
                let error = "Connection closed".to_string();
                signals.set(0, error);
                connected.store(false, atomic::Ordering::Relaxed);
                break;
            }
            let res = res.unwrap();

            if let Err(e) = res {
                let error = format!("Error reading message: {:?}", e);
                signals.set(0, error);
                connected.store(false, atomic::Ordering::Relaxed);
                break;
            }
            let message = res.unwrap();

            let res = match message {
                Message::Binary(m) => {
                    let data = m.as_ref();
                    match data[0] {
                        serialization::TYPE_CONTROL => {
                            let control = ControlMessage::deserialize(data).unwrap(); //TODO handle error
                            match control {
                                ControlMessage::Ack(v) => {
                                    let val_res = values.ack.get(&v);
                                    match val_res {
                                        Some(val) => {
                                            val.acknowledge();
                                            Ok(())
                                        }
                                        None => Err(format!(
                                            "Value with id {} not found for Ack command",
                                            v
                                        )),
                                    }
                                }
                                ControlMessage::Error(err) => {
                                    Err(format!("Error message from UI client: {}", err))
                                }
                                _ => Err(format!(
                                    "Command {} should not be processed here",
                                    control.as_str()
                                )),
                            }
                            // continue;
                        }
                        serialization::TYPE_VALUE => {
                            let id = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                            match values.updated.get(&id) {
                                Some(val) => val.update_value(&data[5..]),
                                None => Err(format!("Value with id {} not found", id)),
                            }
                        }
                        serialization::TYPE_SIGNAL => {
                            let id = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                            match values.updated.get(&id) {
                                Some(val) => val.update_value(&data[5..]),
                                None => Err(format!("Value with id {} not found", id)),
                            }
                        }
                        _ => Err(format!("Unexpected message type: {}", data[0])),
                    }
                }
                _ => Err("Unexpected message format".to_string()),
            };

            if let Err(e) = res {
                let text = format!("Error processing message: {}", e);
                signals.set(0, text);
            }
        }

        // send close signal to writing thread if reading fails
        sender.close();

        // wait for writing thread to finish and return the receiver
        writer_handle.await.unwrap()
    });

    handler
}

async fn writer(
    mut rx: UnboundedReceiver<Option<Bytes>>,
    connected: Arc<AtomicBool>,
    mut websocket: SplitSink<WebSocketStream<TcpStream>, Message>,
    signals: ChangedValues,
) -> UnboundedReceiver<Option<Bytes>> {
    loop {
        // get message from channel
        let message = rx.recv().await.unwrap();

        // check if message is terminate signal
        if message.is_none() {
            let _ = websocket.close().await;
            break;
        }
        let message = message.unwrap();

        // if not connected, stop thread
        if !connected.load(atomic::Ordering::Relaxed) {
            let _ = websocket.close().await;
            break;
        }

        // send message
        let res = websocket.send(Message::Binary(message)).await;
        if let Err(e) = res {
            let error = format!("Error writing message: {:?}", e);
            signals.set(0, error);
            connected.store(false, atomic::Ordering::Relaxed);
            break;
        }
    }
    rx
}
