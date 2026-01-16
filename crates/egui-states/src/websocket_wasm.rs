use bytes::Bytes;
use futures_util::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use std::net::SocketAddrV4;
use ws_stream_wasm::{WsMessage, WsMeta, WsStream};

use egui_states_core::serialization::FastVec;

use crate::handle_message::{MessagesParser, ServerMessage};

pub(crate) async fn build_ws(address: SocketAddrV4) -> Result<(WsClientRead, WsClientSend), ()> {
    let address = format!("ws://{}/ws", address);
    let res = WsMeta::connect(&address, None).await;

    if res.is_err() {
        #[cfg(debug_assertions)]
        log::error!(
            "connecting to server at {:?} failed: {:?}",
            address,
            res.err()
        );
        return Err(());
    }

    // get the socket
    let socket = res.unwrap().1;

    // split the socket
    let (socket_write, socket_read) = socket.split();

    Ok((
        WsClientRead {
            stream: socket_read,
            parser: MessagesParser::empty(),
        },
        WsClientSend { sink: socket_write },
    ))
}

pub(crate) struct WsClientRead {
    stream: SplitStream<WsStream>,
    parser: MessagesParser,
}

impl WsClientRead {
    pub(crate) async fn read(&mut self) -> Result<ServerMessage, &'static str> {
        if let Some(message) = self.parser.next()? {
            return Ok(message);
        }

        match self.stream.next().await {
            Some(message) => match message {
                WsMessage::Binary(data) => {
                    let (parser, message) = MessagesParser::from_bytes(Bytes::from_owner(data))?;
                    self.parser = parser;
                    Ok(message)
                }
                _ => {
                    #[cfg(debug_assertions)]
                    log::error!("Unexpected message from server: {:?}", message);
                    Err("Unexpected message from server")
                }
            },
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
        let message = WsMessage::Binary(data.to_vec());
        match self.sink.send(message).await {
            Ok(_) => Ok(()),
            Err(e) => {
                #[cfg(debug_assertions)]
                log::error!("Sending message to socket failed: {:?}", e);
                Err(())
            }
        }
    }
}
