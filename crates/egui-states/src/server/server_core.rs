use std::net::SocketAddrV4;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::task::JoinHandle;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

use crate::PROTOCOL_VERSION;
use crate::event_async::Event;
use crate::serialization::{ServerHeader, serialize};
use crate::server::sender::{MessageReceiver, MessageSender, SenderData};
use crate::server::server::ServerStatesList;
use crate::server::signals::SignalsManager;
use crate::server::socket_reader::{ClientMessage, SocketReader};

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
            Ok(ClientMessage::Handshake(p, h)) => {
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
                        sender.close();
                        handler.await.expect("joining communication handler failed")
                    }
                    ChannelHolder::Rx(rx) => rx,
                };

                // clean mesage queue and send sync signals
                while !rx.is_empty() {
                    let _ = rx.recv().await;
                }

                // std::thread::sleep(std::time::Duration::from_millis(100));
                connected.store(true, Ordering::Release);
                let mut success = true;
                for v in values.sync.iter() {
                    if let Err(_) = v.sync() {
                        success = false;
                        break;
                    }
                }
                if !success {
                    signals.error("failed to sync value after handshake");
                    connected.store(false, Ordering::Release);
                    holder = ChannelHolder::Rx(rx);
                    continue;
                }
                match serialize(&ServerHeader::Update(0.0)) {
                    Ok(data) => sender.send(data),
                    Err(_) => {
                        signals.error("failed to serialize update message");
                        connected.store(false, Ordering::Release);
                        holder = ChannelHolder::Rx(rx);
                        continue;
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
            Ok(ClientMessage::Value(id, type_id, signal, data)) => match values.values.get(&id) {
                Some(val) => {
                    if let Err(e) = val.update_value(type_id, signal, data) {
                        signals.error(&format!("value updating failed: {}", e));
                    }
                }
                None => signals.error(&format!("value with id {} not found", id)),
            },
            Ok(ClientMessage::Signal(id, type_id, data)) => match values.signals.get(&id) {
                Some(val) => {
                    if let Err(e) = val.update_signal(type_id, data) {
                        signals.error(&format!("signal updating failed: {}", e));
                    }
                }
                None => signals.error(&format!("value with id {} not found", id)),
            },
            Ok(ClientMessage::Handshake(_, _)) => {
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
    rx: MessageReceiver,
    connected: Arc<AtomicBool>,
    mut websocket: SplitSink<WebSocketStream<TcpStream>, Message>,
    signals: SignalsManager,
    reader_handle: tokio::task::JoinHandle<()>,
) -> MessageReceiver {
    let mut data_receiver = DataReceiver::new(rx);
    loop {
        // get message from channel
        match data_receiver.next().await {
            Some(msg) => {
                // if not connected, stop thread
                if !connected.load(Ordering::Acquire) {
                    let _ = websocket.close().await;
                    reader_handle.abort();
                    let _ = reader_handle.await;
                    break;
                }

                // send message
                let data = Message::Binary(msg.to_bytes());
                if let Err(e) = websocket.send(data).await {
                    signals.error(&format!("sending message to client failed: {:?}", e));
                    reader_handle.abort();
                    let _ = reader_handle.await;
                    break;
                }
            }
            // check if message is terminate signal
            None => {
                signals.info("writer is closing connection");
                let _ = websocket.close().await;
                reader_handle.abort();
                let _ = reader_handle.await;
                break;
            }
        }
    }
    data_receiver.finalize()
}

// A helper struct to receive from MessageReceiver and create micro-batches
struct DataReceiver {
    rx: MessageReceiver,
    send_next: Option<SenderData>,
    is_closed: bool,
}

impl DataReceiver {
    fn new(rx: MessageReceiver) -> Self {
        Self {
            rx,
            send_next: None,
            is_closed: false,
        }
    }

    async fn next(&mut self) -> Option<SenderData> {
        // empty send_next first
        if let Some(data) = self.send_next.take() {
            return Some(data);
        }

        if self.is_closed {
            return None;
        }

        match self.rx.recv().await {
            Some(Some((mut msg, send_now))) => match send_now {
                true => Some(msg),
                false => {
                    let mut counter = 0;
                    loop {
                        if msg.len() > MSG_SIZE_THRESHOLD || counter >= MAX_MSG_COUNT {
                            break Some(msg);
                        }

                        match self.rx.try_recv() {
                            Ok(Some((next_msg, send_now))) => match send_now {
                                true => {
                                    self.send_next = Some(next_msg);
                                    break Some(msg);
                                }
                                false => {
                                    msg.extend_from_data(&next_msg);
                                    counter += 1;
                                }
                            },
                            Err(TryRecvError::Empty) => {
                                break Some(msg);
                            }
                            Ok(None) | Err(TryRecvError::Disconnected) => {
                                self.is_closed = true;
                                break Some(msg);
                            }
                        }
                    }
                }
            },
            None | Some(None) => None,
        }
    }

    fn finalize(self) -> MessageReceiver {
        self.rx
    }
}

#[cfg(all(test, feature = "client"))]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4, TcpListener as StdTcpListener};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message;

    use crate::PROTOCOL_VERSION;
    use crate::graphs::{GraphDataInfo, GraphHeader, GraphType};
    use crate::event_async::Event;
    use crate::image::{ImageHeader, ImageType};
    use crate::serialization::{ClientHeader, ServerHeader, to_message};
    use crate::server::graphs::GraphData;
    use crate::server::image::ImageData;
    use crate::server::sender::MessageSender;
    use crate::server::server::{Server, ServerStatesList, SyncTrait};
    use crate::server::signals::SignalsManager;

    fn get_free_addr() -> SocketAddrV4 {
        let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, addr.port())
    }

    struct FailOnceSync(Arc<AtomicUsize>);

    impl SyncTrait for FailOnceSync {
        fn sync(&self) -> Result<(), ()> {
            match self.0.fetch_add(1, Ordering::AcqRel) {
                0 => Err(()),
                _ => Ok(()),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn startup_sync_messages_are_valid() {
        let addr = get_free_addr();
        let mut server = Server::new(addr, None, 1);

        let value_id = server.add_value("value", 1, to_message(&12u32).to_bytes(), false).unwrap();
        let list_id = server.add_vec("list", 2).unwrap();
        let map_id = server.add_map("map", 3).unwrap();
        let image_id = server.add_image("image").unwrap();
        let graphs_id = server.add_graphs("graphs", GraphType::F32).unwrap();

        let states = server.finalize().unwrap();

        states
            .lists
            .get(&list_id)
            .unwrap()
            .append_item(to_message(&5u16).to_bytes(), false)
            .unwrap();
        states
            .maps
            .get(&map_id)
            .unwrap()
            .set_item(to_message(&7u8).to_bytes(), to_message(&9u16).to_bytes(), false)
            .unwrap();

        let image = [10u8, 20, 30, 255];
        states
            .images
            .get(&image_id)
            .unwrap()
            .set_image(
                ImageData {
                    size: [1, 1],
                    stride: 4,
                    contiguous: true,
                    image_type: ImageType::ColorAlpha,
                    data: image.as_ptr(),
                },
                None,
                false,
            )
            .unwrap();

        let graph = [1.0f32, 2.0];
        states
            .graphs
            .get(&graphs_id)
            .unwrap()
            .set(
                0,
                GraphData {
                    graph_type: GraphType::F32,
                    y: graph.as_ptr() as *const u8,
                    x: None,
                    size: std::mem::size_of_val(&graph),
                    count: graph.len(),
                },
                false,
            );

        server.start().unwrap();

        let (mut socket, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
        socket
            .send(Message::Binary(
                ClientHeader::serialize_handshake(PROTOCOL_VERSION, 0).to_bytes(),
            ))
            .await
            .unwrap();

        let mut seen_value = false;
        let mut seen_list = false;
        let mut seen_map = false;
        let mut seen_image = false;
        let mut seen_graph_reset = false;
        let mut seen_graph = false;
        let mut seen_update = false;

        for _ in 0..8 {
            let frame = socket.next().await.unwrap().unwrap();

            let Message::Binary(data) = frame else {
                panic!("unexpected websocket frame: {frame:?}");
            };

            let mut pointer = 0;
            while pointer < data.len() {
                let (header, header_size) = ServerHeader::deserialize(&data[pointer..]).unwrap();
                pointer += header_size;

                match header {
                    ServerHeader::Value(id, _, _, value_size) => {
                        assert_eq!(id, value_id);
                        pointer += value_size as usize;
                        seen_value = true;
                    }
                    ServerHeader::ValueVec(id, _, _, _, value_size) => {
                        assert_eq!(id, list_id);
                        pointer += value_size as usize;
                        seen_list = true;
                    }
                    ServerHeader::ValueMapMap(id, _, _, _, value_size) => {
                        assert_eq!(id, map_id);
                        pointer += value_size as usize;
                        seen_map = true;
                    }
                    ServerHeader::Image(id, _, ImageHeader { image_size, rect, image_type }) => {
                        assert_eq!(id, image_id);
                        assert_eq!(image_size, [1, 1]);
                        assert_eq!(rect, None);
                        assert!(matches!(image_type, ImageType::ColorAlpha));
                        assert_eq!(data.len() - pointer, 4);
                        pointer = data.len();
                        seen_image = true;
                    }
                    ServerHeader::Graph(id, _, GraphHeader::Reset) => {
                        assert_eq!(id, graphs_id);
                        seen_graph_reset = true;
                    }
                    ServerHeader::Graph(
                        id,
                        _,
                        GraphHeader::Set(_, GraphDataInfo { graph_type, is_linear, points }),
                    ) => {
                        assert_eq!(id, graphs_id);
                        assert!(matches!(graph_type, GraphType::F32));
                        assert!(is_linear);
                        assert_eq!(points, 2);
                        assert_eq!(data.len() - pointer, std::mem::size_of_val(&graph));
                        pointer = data.len();
                        seen_graph = true;
                    }
                    ServerHeader::Update(dt) => {
                        assert_eq!(dt, 0.0);
                        seen_update = true;
                    }
                    other => panic!(
                        "unexpected startup message: {:?}",
                        std::mem::discriminant(&other)
                    ),
                }
            }

            if seen_value
                && seen_list
                && seen_map
                && seen_image
                && seen_graph_reset
                && seen_graph
                && seen_update
            {
                break;
            }
        }

        assert!(seen_value);
        assert!(seen_list);
        assert!(seen_map);
        assert!(seen_image);
        assert!(seen_graph_reset);
        assert!(seen_graph);
        assert!(seen_update);

        server.stop();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn server_keeps_accepting_connections_after_sync_failure() {
        let addr = get_free_addr();
        let (sender, rx) = MessageSender::new();
        let connected = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_event = Event::new();
        let signals = SignalsManager::new();
        let failures = Arc::new(AtomicUsize::new(0));

        let mut values = ServerStatesList::default();
        values.sync.push(Arc::new(FailOnceSync(failures.clone())));

        let server_handle = tokio::spawn(super::run(
            sender.clone(),
            rx,
            connected.clone(),
            stop_event.clone(),
            values,
            signals,
            addr,
            None,
        ));

        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
        let (mut first_socket, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
        first_socket
            .send(Message::Binary(
                ClientHeader::serialize_handshake(PROTOCOL_VERSION, 0).to_bytes(),
            ))
            .await
            .unwrap();
        let _ = first_socket.close(None).await;

        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
        let (mut second_socket, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
        second_socket
            .send(Message::Binary(
                ClientHeader::serialize_handshake(PROTOCOL_VERSION, 0).to_bytes(),
            ))
            .await
            .unwrap();

        let frame = second_socket.next().await.unwrap().unwrap();
        let Message::Binary(data) = frame else {
            panic!("unexpected websocket frame: {frame:?}");
        };
        let (header, size) = ServerHeader::deserialize(&data).unwrap();
        assert!(matches!(header, ServerHeader::Update(0.0)));
        assert_eq!(size, data.len());
        assert_eq!(failures.load(Ordering::Acquire), 2);

        stop_event.set();
        sender.close();
        let _ = server_handle.await.unwrap();
    }
}
