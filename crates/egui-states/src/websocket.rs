use futures_util::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use std::net::SocketAddrV4;
use tokio::net::TcpStream;
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use egui_states_core::serialization::FastVec;

pub(crate) async fn build_ws(address: SocketAddrV4) -> Result<(WsClientRead, WsClientSend), ()> {
    let address = format!("ws://{}/ws", address);
    let mut websocket_config = WebSocketConfig::default();
    websocket_config.max_message_size = Some(536870912); // 512 MB
    websocket_config.max_frame_size = Some(536870912); // 512 MB
    let res = connect_async_with_config(&address, Some(websocket_config), false).await;

    if res.is_err() {
        #[cfg(debug_assertions)]
        println!(
            "connecting to server at {:?} failed: {:?}",
            address,
            res.err()
        );
        return Err(());
    }

    // get the socket
    let socket = res.unwrap().0;

    // split the socket
    let (socket_write, socket_read) = socket.split();

    Ok((
        WsClientRead {
            stream: socket_read,
        },
        WsClientSend { sink: socket_write },
    ))
}

pub(crate) struct ReadData(bytes::Bytes);

impl ReadData {
    #[inline]
    pub(crate) fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

pub(crate) struct WsClientRead {
    stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
}

impl WsClientRead {
    pub(crate) async fn read(&mut self) -> Result<ReadData, ()> {
        match self.stream.next().await {
            Some(message) => match message {
                Ok(message) => match message {
                    Message::Binary(data) => Ok(ReadData(data)),
                    Message::Close(_) => Err(()),
                    _ => {
                        #[cfg(debug_assertions)]
                        println!("Unexpected message from server: {:?}", message);
                        Err(())
                    }
                },
                Err(e) => {
                    #[cfg(debug_assertions)]
                    println!("Reading message from server failed: {:?}", e);
                    Err(())
                }
            },
            None => {
                #[cfg(debug_assertions)]
                println!("Connection closed by server");
                Err(())
            }
        }
    }
}

pub(crate) struct WsClientSend {
    sink: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
}

impl WsClientSend {
    #[inline]
    pub(crate) async fn close(&mut self) {
        let _ = self.sink.close().await;
    }

    pub(crate) async fn send(&mut self, data: FastVec<64>) -> Result<(), ()> {
        let message = Message::Binary(data.to_bytes());
        match self.sink.send(message).await {
            Ok(_) => Ok(()),
            Err(e) => {
                #[cfg(debug_assertions)]
                println!("Sending message to socket failed: {:?}", e);
                Err(())
            }
        }
    }
}
