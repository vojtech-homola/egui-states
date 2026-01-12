use std::net::SocketAddrV4;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

use egui_states_core::PROTOCOL_VERSION;
use egui_states_core::event_async::Event;
use egui_states_core::serialization::{ServerHeader, serialize};

use crate::sender::{MessageReceiver, MessageSender};
use crate::server::ServerStatesList;
use crate::signals::SignalsManager;
use crate::socket_reader::{ClientMessage, SocketReader};

const MSG_SIZE_THRESHOLD: usize = 1024 * 1024 * 10; // 10 MB
const MAX_MSG_COUNT: usize = 10;

enum ChannelHolder {
    Transfer(JoinHandle<MessageReceiver>),
    Rx(MessageReceiver),
}

pub(crate) async fn run(
    sender: MessageSender,
    rx: MessageReceiver,
    connected: Arc<AtomicBool>,
    stop_event: Event,
    values: ServerStatesList,
    signals: SignalsManager,
    addr: SocketAddrV4,
    handshake: Option<Vec<u64>>,
) -> MessageReceiver {
    // listen to incoming connections
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            stop_event.clear();
            signals.error(&format!("binding failed: {:?}", e));
            return rx;
        }
    };

    let mut holder = ChannelHolder::Rx(rx);

    loop {
        // check for stop event or incoming connection
        let stream = tokio::select! {
            biased;
            _ = stop_event.wait_clear() => {
                break;
            }
            s = listener.accept() => {
                s
            }
        };

        // check if error accepting connection
        let stream = match stream {
            Ok(s) => s.0,
            Err(e) => {
                signals.error(&format!("accepting connection failed: {:?}", e));
                continue;
            }
        };

        if let Err(e) = stream.set_nodelay(true) {
            signals.error(&format!("failed to set TCP_NODELAY: {:?}", e));
            continue;
        }

        let mut websocket_config = WebSocketConfig::default();
        websocket_config.max_message_size = Some(536870912); // 512 MB
        websocket_config.max_frame_size = Some(536870912); // 512 MB
        let websocket =
            match tokio_tungstenite::accept_async_with_config(stream, Some(websocket_config)).await
            {
                Ok(ws) => ws,
                Err(e) => {
                    signals.error(&format!("websocket handshake failed: {:?}", e));
                    connected.store(false, Ordering::Release);
                    continue;
                }
            };

        let (socket_tx, socket_rx) = websocket.split();
        let mut socket_reader = SocketReader::new(socket_rx);

        match socket_reader.next().await {
            Err(e) => {
                signals.error(e);
                connected.store(false, Ordering::Release);
                continue;
            }
            Ok(ClientMessage::Handshake(p, h, types)) => {
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

                let mut rx = match holder {
                    // disconnect previous client
                    ChannelHolder::Transfer(handler) => {
                        #[cfg(debug_assertions)]
                        signals.debug("terminating previous connection");
                        connected.store(false, Ordering::Release);
                        for (_, v) in &values.enable {
                            v.enable(false);
                        }
                        sender.close();
                        handler.await.expect("joining communication handler failed")
                    }
                    ChannelHolder::Rx(rx) => rx,
                };

                // clean mesage queue and send sync signals
                while !rx.is_empty() {
                    let _ = rx.recv().await;
                }

                for (id, client_type) in types {
                    if let Some(server_type) = values.types.get(&id) {
                        if client_type == *server_type {
                            if let Some(v) = values.enable.get(&id) {
                                v.enable(true);
                            }
                        }
                    }
                }

                // std::thread::sleep(std::time::Duration::from_millis(100));
                connected.store(true, Ordering::Release);
                for v in values.sync.iter() {
                    v.sync();
                }
                match serialize(&ServerHeader::Update(0.0)) {
                    Ok(data) => sender.send(data),
                    Err(_) => {
                        signals.error("failed to serialize update message");
                        connected.store(false, Ordering::Release);
                        holder = ChannelHolder::Rx(rx);
                        break;
                    }
                }

                let reader_handler = tokio::spawn(reader(
                    socket_reader,
                    connected.clone(),
                    signals.clone(),
                    values.clone(),
                    sender.clone(),
                ));
                let handler = tokio::spawn(writer(
                    rx,
                    connected.clone(),
                    socket_tx,
                    signals.clone(),
                    reader_handler,
                ));

                holder = ChannelHolder::Transfer(handler);
            }
            Ok(_) => {
                signals.error("expected handshake message from client");
                connected.store(false, Ordering::Release);
                continue;
            }
        }
    }

    match holder {
        // disconnect previous client
        ChannelHolder::Transfer(handler) => {
            #[cfg(debug_assertions)]
            signals.debug("terminating previous connection");
            connected.store(false, Ordering::Release);
            for (_, v) in &values.enable {
                v.enable(false);
            }
            sender.close();
            handler.await.expect("joining communication handler failed")
        }
        ChannelHolder::Rx(rx) => rx,
    }
}

async fn reader(
    mut socket_rx: SocketReader,
    connected: Arc<AtomicBool>,
    signals: SignalsManager,
    values: ServerStatesList,
    sender: MessageSender,
) {
    loop {
        // read the message
        let result_message = socket_rx.next().await;

        // check if not connected
        if !connected.load(Ordering::Acquire) {
            #[cfg(debug_assertions)]
            signals.debug("read thread is closing");
            signals.reset();
            break;
        }

        match result_message {
            Err(e) => {
                signals.error(e);
                connected.store(false, Ordering::Release);
                signals.reset();
                break;
            }
            Ok(ClientMessage::Ack(id)) => {
                let val_res = values.ack.get(&id);
                match val_res {
                    Some(val) => {
                        val.acknowledge();
                    }
                    None => {
                        signals.error(&format!("value with id {} not found for Acknowledge", id))
                    }
                }
            }
            Ok(ClientMessage::Value(id, signal, data)) => match values.values.get(&id) {
                Some(val) => {
                    if let Err(e) = val.update_value(signal, data) {
                        signals.error(&format!("value updating failed: {}", e));
                    }
                }
                None => signals.error(&format!("value with id {} not found", id)),
            },
            Ok(ClientMessage::Signal(id, data)) => match values.signals.get(&id) {
                Some(val) => {
                    if let Err(e) = val.update_signal(data) {
                        signals.error(&format!("signal updating failed: {}", e));
                    }
                }
                None => signals.error(&format!("value with id {} not found", id)),
            },
            Ok(ClientMessage::Error(err)) => {
                signals.error(&format!("Error message from client: {}", err));
            }
            Ok(ClientMessage::Handshake(_, _, _)) => {
                signals.error("unexpected handshake message after connection established");
            }
        }
    }

    // acknowledge all pending values
    for v in values.ack.values() {
        v.acknowledge();
    }

    // send close signal to writing thread if reading fails
    #[cfg(debug_assertions)]
    signals.debug("terminating write thread");
    sender.close();
}

async fn writer(
    mut rx: MessageReceiver,
    connected: Arc<AtomicBool>,
    mut websocket: SplitSink<WebSocketStream<TcpStream>, Message>,
    signals: SignalsManager,
    reader_handle: tokio::task::JoinHandle<()>,
) -> MessageReceiver {
    loop {
        // get message from channel
        let msg = match rx.recv().await {
            Some(Some(m)) => m,
            // check if message is terminate signal
            _ => {
                signals.info("writer is closing connection");
                let _ = websocket.close().await;
                reader_handle.abort();
                let _ = reader_handle.await;
                break;
            }
        };

        // if not connected, stop thread
        if !connected.load(Ordering::Acquire) {
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
