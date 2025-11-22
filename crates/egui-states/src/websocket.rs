use futures_util::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use std::net::{TcpStream, Ipv4Addr, SocketAddrV4};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub(crate) fn build_ws(address: ) -> Result<(WsClientRead, WsClientWrite), ()> {}

pub(crate) struct WsClientRead {
    stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
}

pub(crate) struct WsClientWrite {
    sink: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
}
