use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Instant, timeout_at};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};

use crate::PROTOCOL_VERSION;
use crate::event::Event;
use crate::serialization::{
    MAX_MSG_COUNT, MSG_SIZE_THRESHOLD, ServerHeader, WEBSOCKET_MAX_SIZE, serialize,
};
use crate::server_core::sender::{MessageReceiver, MessageSender, SenderData};
use crate::server_core::server::ServerStatesList;
use crate::server_core::signals::SignalsManager;
use crate::server_core::socket_reader::{ClientMessage, SocketReader};
use crate::server_core::web::{self, Dispatch, WebSite, WebSlot};

enum ChannelHolder {
    Transfer(JoinHandle<MessageReceiver>),
    Rx(MessageReceiver),
}

#[derive(Clone)]
pub(crate) struct Handshake {
    pub version: Option<u64>,
    pub token: Option<String>,
}

/// Bounds how long a new connection may take to send its request head and
/// complete the WebSocket and protocol handshakes.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// A connection that passed the protocol handshake and may replace the
/// current client.
struct ClientReady {
    socket_tx: SplitSink<WebSocketStream<TcpStream>, Message>,
    socket_reader: SocketReader,
    peer_addr: String,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    listener: TcpListener,
    sender: MessageSender,
    rx: MessageReceiver,
    connected: Arc<AtomicBool>,
    stop_event: Event,
    values: ServerStatesList,
    signals: SignalsManager,
    handshake: Handshake,
    web: WebSlot,
) -> MessageReceiver {
    let mut holder = ChannelHolder::Rx(rx);
    // Connections still handshaking or receiving the web page. They run
    // concurrently so an idle connection cannot hold up the others; dropping
    // the set on shutdown aborts them.
    let mut pending: JoinSet<Option<ClientReady>> = JoinSet::new();

    loop {
        // check for stop event, a finished handshake or incoming connection
        let client = tokio::select! {
            biased;
            _ = stop_event.wait_clear_async() => {
                break;
            }
            Some(result) = pending.join_next(), if !pending.is_empty() => {
                match result {
                    Ok(Some(client)) => client,
                    Ok(None) => continue,
                    Err(e) => {
                        if e.is_panic() {
                            signals.error("connection task panicked");
                        }
                        continue;
                    }
                }
            }
            s = listener.accept() => {
                match s {
                    Ok((stream, _)) => {
                        if let Err(e) = stream.set_nodelay(true) {
                            signals.error(format!("failed to set TCP_NODELAY: {:?}", e));
                            continue;
                        }
                        let site = web.read().clone();
                        pending.spawn(accept_client(
                            stream,
                            site,
                            handshake.clone(),
                            signals.clone(),
                        ));
                    }
                    Err(e) => {
                        signals.error(format!("accepting connection failed: {:?}", e));
                    }
                }
                continue;
            }
        };

        let ClientReady {
            socket_tx,
            socket_reader,
            peer_addr,
        } = client;

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
        let failure = if values.sync.iter().any(|v| v.sync().is_err()) {
            Some("failed to sync value after handshake")
        } else {
            match serialize(&ServerHeader::Update(0.0)) {
                Ok(data) => {
                    sender.send(data);
                    None
                }
                Err(_) => Some("failed to serialize update message"),
            }
        };
        if let Some(error) = failure {
            // Drop only this client; the server keeps accepting connections.
            // The partial sync may have left producers waiting for this
            // client's acknowledgements, so release them.
            signals.error(error);
            connected.store(false, Ordering::Release);
            for v in values.ack.values() {
                v.reset();
            }
            holder = ChannelHolder::Rx(rx);
            continue;
        }

        signals.on_connect(peer_addr);

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
            values.clone(),
            reader_handler,
        ));

        holder = ChannelHolder::Transfer(handler);
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

/// Takes a new connection up to a passed protocol handshake.
///
/// With a web page configured, plain HTTP requests are answered here and
/// yield `None`. Failures never touch the current client's connection.
async fn accept_client(
    stream: TcpStream,
    site: Option<Arc<WebSite>>,
    handshake: Handshake,
    signals: SignalsManager,
) -> Option<ClientReady> {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;

    let stream = match site {
        None => stream,
        Some(site) => match timeout_at(deadline, web::dispatch(stream)).await {
            Ok(Ok(Dispatch::WebSocket(stream))) => stream,
            Ok(Ok(Dispatch::Http(stream, request))) => {
                // Browsers abort downloads on navigation, so failures here are
                // not server errors.
                if let Err(_e) = web::serve(stream, request, &site).await {
                    #[cfg(debug_assertions)]
                    signals.debug(format!("serving web page failed: {:?}", _e));
                }
                return None;
            }
            // Browsers open speculative connections that may stay idle and be
            // closed unused, so these are not reported.
            Ok(Ok(Dispatch::Closed)) | Err(_) => return None,
            Ok(Err(e)) => {
                signals.error(format!("reading request failed: {:?}", e));
                return None;
            }
        },
    };

    match timeout_at(deadline, handshake_client(stream, &handshake, &signals)).await {
        Ok(client) => client,
        Err(_) => {
            signals.warning("client handshake timed out");
            None
        }
    }
}

async fn handshake_client(
    stream: TcpStream,
    handshake: &Handshake,
    signals: &SignalsManager,
) -> Option<ClientReady> {
    let mut websocket_config = WebSocketConfig::default();
    websocket_config.max_message_size = Some(WEBSOCKET_MAX_SIZE);
    websocket_config.max_frame_size = Some(WEBSOCKET_MAX_SIZE);
    let websocket =
        match tokio_tungstenite::accept_async_with_config(stream, Some(websocket_config)).await {
            Ok(ws) => ws,
            Err(e) => {
                signals.error(format!("websocket handshake failed: {:?}", e));
                return None;
            }
        };

    let peer_addr = websocket
        .get_ref()
        .peer_addr()
        .map_or("".to_string(), |addr| addr.to_string());
    let (socket_tx, socket_rx) = websocket.split();
    let mut socket_reader = SocketReader::new(socket_rx);

    match socket_reader.next().await {
        Err(e) => {
            if let Some(e) = e {
                signals.error(e);
            }
            None
        }
        Ok(ClientMessage::Handshake(p, v, h)) => {
            if let Err(e) = check_handshake(handshake, p, v, h) {
                signals.warning(&e);
                return None;
            }
            Some(ClientReady {
                socket_tx,
                socket_reader,
                peer_addr,
            })
        }
        Ok(_) => {
            signals.error("expected handshake message from client");
            None
        }
    }
}

fn check_handshake(
    server_handshake: &Handshake,
    client_protocol: u16,
    version: Option<u64>,
    token: Option<String>,
) -> Result<(), String> {
    if client_protocol != PROTOCOL_VERSION {
        let message = format!(
            "attempted to connect with wrong protocol version: expected {}, got {}",
            PROTOCOL_VERSION, client_protocol
        );
        return Err(message);
    }

    if let Some(server_version) = server_handshake.version {
        match version {
            Some(client_version) => {
                if client_version != server_version {
                    let message = format!(
                        "attempted to connect with wrong version: expected {}, got {}",
                        server_version, client_version
                    );
                    return Err(message);
                }
            }
            _ => return Err("client version missing".to_string()),
        }
    }

    if let Some(server_token) = &server_handshake.token {
        match token {
            Some(client_token) => {
                if client_token != *server_token {
                    return Err("attempted to connect with wrong token".to_string());
                }
            }
            _ => return Err("client token missing".to_string()),
        }
    }

    Ok(())
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
                if let Some(e) = e {
                    signals.error(e);
                }
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
                        signals.error(format!("value with id {} not found for Acknowledge", id))
                    }
                }
            }
            Ok(ClientMessage::Value(id, type_id, signal, data)) => match values.values.get(&id) {
                Some(val) => {
                    if let Err(e) = val.update_value(type_id, signal, data) {
                        signals.error(format!("value updating failed: {}", e));
                    }
                }
                None => signals.error(format!("value with id {} not found", id)),
            },
            Ok(ClientMessage::Signal(id, type_id, data)) => match values.signals.get(&id) {
                Some(val) => {
                    if let Err(e) = val.update_signal(type_id, data) {
                        signals.error(format!("signal updating failed: {}", e));
                    }
                }
                None => signals.error(format!("value with id {} not found", id)),
            },
            Ok(ClientMessage::Message(data)) => {
                signals.client_message(data);
            }
            Ok(ClientMessage::Handshake(_, _, _)) => {
                signals.error("unexpected handshake message after connection established");
            }
        }
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
    values: ServerStatesList,
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
                let data = Message::Binary(msg.into_bytes());
                if let Err(e) = websocket.send(data).await {
                    signals.error(format!("sending message to client failed: {:?}", e));
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

    connected.store(false, Ordering::Release);
    signals.on_disconnect();

    // reset all pendings values
    for v in values.ack.values() {
        v.reset();
    }

    data_receiver.finalize()
}

// A helper struct to receive from MessageReceiver and create micro-batches
struct DataReceiver {
    rx: MessageReceiver,
    send_next: Option<(SenderData, bool)>,
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
        let (mut msg, send_now) = match self.send_next.take() {
            Some(next) => next,
            None if self.is_closed => return None,
            None => match self.rx.recv().await {
                Some(Some(next)) => next,
                None | Some(None) => return None,
            },
        };

        if send_now {
            return Some(msg);
        }

        let mut counter = 1;
        loop {
            if msg.len() > MSG_SIZE_THRESHOLD || counter >= MAX_MSG_COUNT {
                return Some(msg);
            }

            match self.rx.try_recv() {
                Ok(Some((next_msg, true))) => {
                    self.send_next = Some((next_msg, true));
                    return Some(msg);
                }
                Ok(Some((next_msg, false))) => {
                    if msg
                        .len()
                        .checked_add(next_msg.len())
                        .is_none_or(|size| size > MSG_SIZE_THRESHOLD)
                    {
                        self.send_next = Some((next_msg, false));
                        return Some(msg);
                    }
                    msg.extend_from_data(&next_msg);
                    counter += 1;
                }
                Err(TryRecvError::Empty) => return Some(msg),
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    self.is_closed = true;
                    return Some(msg);
                }
            }
        }
    }

    fn finalize(self) -> MessageReceiver {
        self.rx
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::unbounded_channel;

    use super::DataReceiver;
    use crate::serialization::{FastVec, MAX_MSG_COUNT, MSG_SIZE_THRESHOLD};

    fn data(size: usize) -> FastVec<32> {
        FastVec::Heap((0..size).map(|i| (i % 251) as u8).collect())
    }

    #[tokio::test]
    async fn data_receiver_does_not_batch_past_size_threshold() {
        let (tx, rx) = unbounded_channel();
        tx.send(Some((data(MSG_SIZE_THRESHOLD - 10), false)))
            .unwrap();
        tx.send(Some((data(11), false))).unwrap();
        drop(tx);

        let mut receiver = DataReceiver::new(rx);
        assert_eq!(
            receiver.next().await.unwrap().into_bytes(),
            data(MSG_SIZE_THRESHOLD - 10).into_bytes()
        );
        assert_eq!(
            receiver.next().await.unwrap().into_bytes(),
            data(11).into_bytes()
        );
        assert!(receiver.next().await.is_none());
    }

    #[tokio::test]
    async fn data_receiver_batches_up_to_size_threshold() {
        let (tx, rx) = unbounded_channel();
        tx.send(Some((data(MSG_SIZE_THRESHOLD - 10), false)))
            .unwrap();
        tx.send(Some((data(10), false))).unwrap();
        drop(tx);

        let mut receiver = DataReceiver::new(rx);
        let mut expected = data(MSG_SIZE_THRESHOLD - 10).into_bytes().to_vec();
        expected.extend_from_slice(&data(10).into_bytes());
        assert_eq!(
            receiver.next().await.unwrap().into_bytes().as_ref(),
            expected
        );
        assert!(receiver.next().await.is_none());
    }

    #[tokio::test]
    async fn data_receiver_batches_deferred_non_immediate_message() {
        let (tx, rx) = unbounded_channel();
        tx.send(Some((data(MSG_SIZE_THRESHOLD - 10), false)))
            .unwrap();
        tx.send(Some((data(11), false))).unwrap();
        tx.send(Some((data(5), false))).unwrap();
        drop(tx);

        let mut receiver = DataReceiver::new(rx);
        assert_eq!(
            receiver.next().await.unwrap().into_bytes(),
            data(MSG_SIZE_THRESHOLD - 10).into_bytes()
        );
        let mut expected = data(11).into_bytes().to_vec();
        expected.extend_from_slice(&data(5).into_bytes());
        assert_eq!(
            receiver.next().await.unwrap().into_bytes().as_ref(),
            expected
        );
        assert!(receiver.next().await.is_none());
    }

    #[tokio::test]
    async fn data_receiver_keeps_immediate_message_separate() {
        let (tx, rx) = unbounded_channel();
        tx.send(Some((data(10), false))).unwrap();
        tx.send(Some((data(11), true))).unwrap();
        tx.send(Some((data(5), false))).unwrap();
        drop(tx);

        let mut receiver = DataReceiver::new(rx);
        assert_eq!(
            receiver.next().await.unwrap().into_bytes(),
            data(10).into_bytes()
        );
        assert_eq!(
            receiver.next().await.unwrap().into_bytes(),
            data(11).into_bytes()
        );
        assert_eq!(
            receiver.next().await.unwrap().into_bytes(),
            data(5).into_bytes()
        );
        assert!(receiver.next().await.is_none());
    }

    #[tokio::test]
    async fn data_receiver_respects_message_count_limit() {
        let (tx, rx) = unbounded_channel();
        for marker in 0..MAX_MSG_COUNT + 2 {
            tx.send(Some((FastVec::Heap(vec![marker as u8]), false)))
                .unwrap();
        }
        drop(tx);

        let mut receiver = DataReceiver::new(rx);
        assert_eq!(
            receiver.next().await.unwrap().into_bytes().as_ref(),
            (0..MAX_MSG_COUNT as u8).collect::<Vec<_>>()
        );
        assert_eq!(
            receiver.next().await.unwrap().into_bytes().as_ref(),
            [MAX_MSG_COUNT as u8, MAX_MSG_COUNT as u8 + 1]
        );
        assert!(receiver.next().await.is_none());
    }
}

#[cfg(test)]
mod run_tests {
    use std::net::{Ipv4Addr, SocketAddr};
    use std::sync::atomic::AtomicUsize;

    use parking_lot::RwLock;
    use tokio_tungstenite::MaybeTlsStream;

    use super::*;
    use crate::serialization::ClientHeader;
    use crate::server_core::server::{Acknowledge, SyncTrait};

    /// Fails only its first connection sync.
    struct FailFirstSync(AtomicUsize);

    impl SyncTrait for FailFirstSync {
        fn sync(&self) -> Result<(), ()> {
            match self.0.fetch_add(1, Ordering::AcqRel) {
                0 => Err(()),
                _ => Ok(()),
            }
        }
    }

    struct ResetProbe(AtomicBool);

    impl Acknowledge for ResetProbe {
        fn acknowledge(&self) {}

        fn reset(&self) {
            self.0.store(true, Ordering::Release);
        }
    }

    async fn connect(addr: SocketAddr) -> WebSocketStream<MaybeTlsStream<TcpStream>> {
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        let handshake = ClientHeader::Handshake(PROTOCOL_VERSION, None, None);
        let bytes = postcard::to_stdvec(&handshake).unwrap();
        ws.send(Message::Binary(bytes.into())).await.unwrap();
        ws
    }

    #[test]
    fn failed_sync_drops_only_that_client() {
        if crate::test_support::isolated() {
            return;
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (sender, rx) = MessageSender::new();
            let connected = Arc::new(AtomicBool::new(false));
            let stop_event = Event::new();
            let probe = Arc::new(ResetProbe(AtomicBool::new(false)));
            let mut values = ServerStatesList::default();
            values.ack.insert(1, probe.clone());
            values
                .sync
                .push(Arc::new(FailFirstSync(AtomicUsize::new(0))));
            let server = tokio::spawn(run(
                listener,
                sender.clone(),
                rx,
                connected.clone(),
                stop_event.clone(),
                values,
                SignalsManager::new(),
                Handshake {
                    version: None,
                    token: None,
                },
                Arc::new(RwLock::new(None)),
            ));
            let deadline = Duration::from_secs(5);

            let mut rejected = connect(addr).await;
            let closed = tokio::time::timeout(deadline, rejected.next())
                .await
                .expect("the client whose sync failed was not dropped");
            assert!(!matches!(closed, Some(Ok(Message::Binary(_)))));
            assert!(
                probe.0.load(Ordering::Acquire),
                "pending states were not reset"
            );
            assert!(!connected.load(Ordering::Acquire));

            let mut accepted = connect(addr).await;
            let update = tokio::time::timeout(deadline, accepted.next())
                .await
                .expect("the next client was not synchronized");
            assert!(matches!(update, Some(Ok(Message::Binary(_)))));
            assert!(connected.load(Ordering::Acquire));

            connected.store(false, Ordering::Release);
            stop_event.set();
            sender.close();
            tokio::time::timeout(deadline, server)
                .await
                .expect("the server did not stop")
                .unwrap();
        });
    }
}

#[cfg(test)]
mod handshake_tests {
    use super::*;

    #[test]
    fn required_handshake_fields_reject_missing_or_wrong_credentials() {
        let settings = Handshake {
            version: Some(17),
            token: Some("secret".into()),
        };
        for (protocol, version, token, diagnostic) in [
            (0, Some(17), Some("secret"), "protocol version"),
            (
                PROTOCOL_VERSION + 1,
                Some(17),
                Some("secret"),
                "protocol version",
            ),
            (PROTOCOL_VERSION, None, Some("secret"), "version missing"),
            (PROTOCOL_VERSION, Some(18), Some("secret"), "wrong version"),
            (PROTOCOL_VERSION, Some(17), None, "token missing"),
            (PROTOCOL_VERSION, Some(17), Some("wrong"), "wrong token"),
        ] {
            let error = check_handshake(&settings, protocol, version, token.map(str::to_owned))
                .unwrap_err();
            assert!(error.contains(diagnostic), "{error}");
            assert!(
                check_handshake(&settings, PROTOCOL_VERSION, Some(17), Some("secret".into()))
                    .is_ok()
            );
        }
    }
}
