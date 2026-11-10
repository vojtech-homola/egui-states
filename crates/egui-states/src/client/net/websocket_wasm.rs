use bytes::Bytes;
use futures_util::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use ws_stream_wasm::{WsMessage, WsMeta, WsStream};

use crate::client::messages::{ServerFrames, ServerReader};
use crate::protocol::FastVec;

pub(crate) async fn build_ws(address: &str) -> Result<(WsClientRead, WsClientSend), String> {
    let res = WsMeta::connect(address, None).await;

    let socket = match res {
        Ok(res) => res.1,
        Err(e) => return Err(format!("Connecting to server at {} failed: {}", address, e)),
    };

    // split the socket
    let (socket_write, socket_read) = socket.split();

    Ok((
        ServerReader::new(WsFrames(socket_read)),
        WsClientSend { sink: socket_write },
    ))
}

pub(crate) type WsClientRead = ServerReader<WsFrames>;

pub(crate) struct WsFrames(SplitStream<WsStream>);

impl ServerFrames for WsFrames {
    async fn next_frame(&mut self) -> Result<Bytes, &'static str> {
        match self.0.next().await {
            Some(WsMessage::Binary(data)) => Ok(Bytes::from_owner(data)),
            Some(_message) => {
                #[cfg(debug_assertions)]
                log::error!("Unexpected message from server: {:?}", _message);
                Err("Unexpected message from server")
            }
            None => {
                #[cfg(debug_assertions)]
                log::info!("Connection closed by server");
                Err("Connection closed by server")
            }
        }
    }
}

pub(crate) struct WsClientSend {
    sink: SplitSink<WsStream, WsMessage>,
}

impl WsClientSend {
    #[inline]
    pub(crate) async fn close(&mut self) {
        let _ = self.sink.close().await;
    }

    pub(crate) async fn send(&mut self, data: FastVec<64>) -> Result<(), ()> {
        let message = WsMessage::Binary(data.into_vec());
        match self.sink.send(message).await {
            Ok(_) => Ok(()),
            Err(e) => {
                #[cfg(debug_assertions)]
                log::error!("Sending message to socket failed: {:?}", e);
                let _ = e;
                Err(())
            }
        }
    }
}
