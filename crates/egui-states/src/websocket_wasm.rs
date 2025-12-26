use futures_util::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use std::net::SocketAddrV4;
use ws_stream_wasm::{WsMessage, WsMeta, WsStream};

use egui_states_core::serialization::MessageData;

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
        },
        WsClientSend { sink: socket_write },
    ))
}

pub(crate) struct ReadData(Vec<u8>);

impl ReadData {
    #[inline]
    pub(crate) fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

pub(crate) struct WsClientRead {
    stream: SplitStream<WsStream>,
}

impl WsClientRead {
    pub(crate) async fn read(&mut self) -> Result<ReadData, ()> {
        match self.stream.next().await {
            Some(message) => match message {
                WsMessage::Binary(data) => Ok(ReadData(data)),
                _ => {
                    #[cfg(debug_assertions)]
                    log::error!("Unexpected message from server: {:?}", message);
                    Err(())
                }
            },
            None => {
                #[cfg(debug_assertions)]
                log::info!("Connection closed by server");
                Err(())
            }
        }
    }
}

pub(crate) struct WsClientSend {
    sink: SplitSink<WsStream, WsMessage>,
}

impl WsClientSend {
    #[inline]
    pub(crate) async fn flush(&mut self) {
        let _ = self.sink.flush().await;
    }

    pub(crate) async fn send(&mut self, data: MessageData) -> Result<(), ()> {
        let message = match data {
            MessageData::Heap(vec) => WsMessage::Binary(vec),
            MessageData::Stack(vec) => WsMessage::Binary(vec.to_vec()),
        };

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
