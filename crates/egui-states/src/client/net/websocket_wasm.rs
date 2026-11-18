use std::pin::pin;
use std::time::Duration;

use bytes::Bytes;
use futures_util::future::{Either, select};
use futures_util::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use pharos::{Events, Observable, ObserveConfig};
use ws_stream_wasm::{WsEvent, WsMessage, WsMeta, WsStream};

use crate::client::messages::{ReadError, ServerFrames, ServerReader};
use crate::client::net::timer;
use crate::protocol::FastVec;

/// How long a closed stream waits for the browser's close event, which
/// carries the server's close code and reason.
const CLOSE_EVENT_WAIT: Duration = Duration::from_secs(1);

pub(crate) async fn build_ws(address: &str) -> Result<(WsClientRead, WsClientSend), String> {
    let res = WsMeta::connect(address, None).await;

    let (mut meta, socket) = match res {
        Ok(res) => res,
        Err(e) => return Err(format!("Connecting to server at {} failed: {}", address, e)),
    };
    // The close event reaches only observers registered before it fires.
    let closes = meta
        .observe(ObserveConfig::default().filter(WsEvent::is_closed))
        .await
        .ok();

    // split the socket
    let (socket_write, socket_read) = socket.split();

    Ok((
        ServerReader::new(WsFrames {
            stream: socket_read,
            closes,
            _meta: meta,
        }),
        WsClientSend { sink: socket_write },
    ))
}

pub(crate) type WsClientRead = ServerReader<WsFrames>;

pub(crate) struct WsFrames {
    stream: SplitStream<WsStream>,
    /// The browser's close events of this socket.
    closes: Option<Events<WsEvent>>,
    /// Owns the browser socket and the source of its events.
    _meta: WsMeta,
}

impl WsFrames {
    /// The code and reason the socket was closed with, waiting briefly for the
    /// browser to report them: the stream ends as soon as closing begins.
    async fn close_frame(&mut self) -> Option<(u16, String)> {
        let closes = self.closes.as_mut()?;
        let next = pin!(closes.next());
        match select(next, timer::sleep(CLOSE_EVENT_WAIT)).await {
            Either::Left((Some(WsEvent::Closed(event)), _)) => Some((event.code, event.reason)),
            _ => None,
        }
    }
}

impl ServerFrames for WsFrames {
    async fn next_frame(&mut self) -> Result<Bytes, ReadError> {
        match self.stream.next().await {
            Some(WsMessage::Binary(data)) => Ok(Bytes::from_owner(data)),
            Some(_message) => {
                #[cfg(debug_assertions)]
                log::error!("Unexpected message from server: {:?}", _message);
                Err(ReadError::Failed("Unexpected message from server"))
            }
            None => {
                #[cfg(debug_assertions)]
                log::info!("Connection closed by server");
                Err(ReadError::Closed(self.close_frame().await))
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

    /// Sends `data`; the error describes why it could not be sent.
    pub(crate) async fn send(&mut self, data: FastVec<64>) -> Result<(), String> {
        let message = WsMessage::Binary(data.into_vec());
        self.sink.send(message).await.map_err(|e| {
            #[cfg(debug_assertions)]
            log::error!("Sending message to socket failed: {:?}", e);
            e.to_string()
        })
    }
}
