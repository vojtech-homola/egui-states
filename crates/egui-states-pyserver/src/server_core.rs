use std::net::SocketAddrV4;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, atomic};

use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Bytes, Message, protocol::WebSocketConfig};

use egui_states_core::controls::ControlMessage;
use egui_states_core::event_async::Event;
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
        start_event.wait().await;

        // listen to incoming connections
        let listener = TcpListener::bind(addr).await;
        if let Err(e) = listener {
            signals.error(&format!("binding failed: {:?}", e));
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
            signals.error(&format!("accepting connection failed: {:?}", e));
            continue;
        }
        let stream = stream.unwrap().0;

        let mut websocket_config = WebSocketConfig::default();
        websocket_config.max_message_size = Some(536870912); // 512 MB
        websocket_config.max_frame_size = Some(536870912); // 512 MB
        let websocket_res =
            tokio_tungstenite::accept_async_with_config(stream, Some(websocket_config)).await;
        if let Err(e) = websocket_res {
            signals.error(&format!("websocket handshake failed: {:?}", e));
            connected.store(false, atomic::Ordering::Relaxed);
            continue;
        }
        let mut websocket = websocket_res.unwrap();

        // read the message
        let res = websocket.next().await;
        if res.is_none() {
            signals.error("reading initial message failed");
            connected.store(false, atomic::Ordering::Relaxed);
            continue;
        }

        let res = res.unwrap();
        if let Err(e) = res {
            signals.error(&format!("reading initial message failed: {:?}", e));
            connected.store(false, atomic::Ordering::Relaxed);
            continue;
        }

        if let Message::Binary(message) = res.unwrap() {
            let data = message.as_ref();
            if data[0] == serialization::TYPE_CONTROL {
                let control = ControlMessage::deserialize(data);
                if control.is_err() {
                    signals.error(&format!("deserializing initial message failed"));
                    continue;
                }
                let control = control.unwrap();

                if let ControlMessage::Handshake(v, h) = control {
                    if v != version {
                        let message = format!(
                            "attempted to connect with different version: {}, version {} is required.",
                            v, version
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

                    let mut rx = match holder {
                        // disconnect previous client
                        ChannelHolder::Transfer(handler) => {
                            #[cfg(debug_assertions)]
                            signals.debug("terminating previous connection");
                            connected.store(false, atomic::Ordering::Relaxed);
                            sender.close();
                            let rx = handler.await.unwrap();
                            rx
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
    let (socket_tx, mut socket_rx) = websocket.split();

    let read_connected = connected.clone();
    let read_signals = signals.clone();
    let read_values = values.clone();
    let read_sender = sender.clone();

    let reader_handler = tokio::spawn(async move {
        loop {
            // read the message
            let res = socket_rx.next().await;

            // check if not connected
            if !read_connected.load(atomic::Ordering::Relaxed) {
                #[cfg(debug_assertions)]
                read_signals.debug("read thread is closing");
                return;
            }

            if res.is_none() {
                read_signals.info("connection was closed by the client");
                read_connected.store(false, atomic::Ordering::Relaxed);
                break;
            }
            let res = res.unwrap();

            if let Err(e) = res {
                read_signals.error(&format!("reading message from client failed: {:?}", e));
                read_connected.store(false, atomic::Ordering::Relaxed);
                break;
            }

            match res.unwrap() {
                Message::Binary(m) => {
                    let data = m.as_ref();
                    match data[0] {
                        serialization::TYPE_CONTROL => {
                            let control = ControlMessage::deserialize(data).unwrap(); //TODO handle error
                            match control {
                                ControlMessage::Ack(v) => {
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
                                ControlMessage::Error(err) => {
                                    read_signals
                                        .error(&format!("Error message from client: {}", err));
                                }
                                _ => read_signals.error(&format!(
                                    "Command {} should not be processed here",
                                    control.as_str()
                                )),
                            }
                        }
                        serialization::TYPE_VALUE => {
                            let id = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                            match read_values.updated.get(&id) {
                                Some(val) => {
                                    if let Err(e) = val.update_value(&data[5..]) {
                                        read_signals.error(&format!(
                                            "updating value with id {} failed: {}",
                                            id, e
                                        ));
                                    }
                                }
                                None => {
                                    read_signals.error(&format!("value with id {} not found", id))
                                }
                            }
                        }
                        serialization::TYPE_SIGNAL => {
                            let id = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                            match read_values.updated.get(&id) {
                                Some(val) => {
                                    if let Err(e) = val.update_value(&data[5..]) {
                                        read_signals.error(&format!(
                                            "updating signal with id {} failed: {}",
                                            id, e
                                        ));
                                    }
                                }
                                None => {
                                    read_signals.error(&format!("value with id {} not found", id))
                                }
                            }
                        }
                        _ => read_signals.error(&format!("Unexpected message type: {}", data[0])),
                    }
                }
                _ => read_signals.error("Unexpected message format"),
            };
        }

        // send close signal to writing thread if reading fails
        #[cfg(debug_assertions)]
        read_signals.debug("terminating write thread");
        read_sender.close();
    });

    tokio::spawn(writer(rx, connected, socket_tx, signals, reader_handler))
}

async fn writer(
    mut rx: UnboundedReceiver<Option<Bytes>>,
    connected: Arc<AtomicBool>,
    mut websocket: SplitSink<WebSocketStream<TcpStream>, Message>,
    signals: ChangedValues,
    reader_handle: tokio::task::JoinHandle<()>,
) -> UnboundedReceiver<Option<Bytes>> {
    loop {
        // get message from channel
        let message = rx.recv().await.unwrap();

        // check if message is terminate signal
        if message.is_none() {
            signals.info("writer is closing connection");
            let _ = websocket.close().await;
            reader_handle.abort();
            let _ = reader_handle.await;
            break;
        }

        // if not connected, stop thread
        if !connected.load(atomic::Ordering::Relaxed) {
            let _ = websocket.close().await;
            reader_handle.abort();
            let _ = reader_handle.await;
            break;
        }

        // send message
        let res = websocket.send(Message::Binary(message.unwrap())).await;
        if let Err(e) = res {
            signals.error(&format!("sending message to client failed: {:?}", e));
            reader_handle.abort();
            let _ = reader_handle.await;
            break;
        }
    }
    rx
}
