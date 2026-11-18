//! The byte stream of an accepted connection, plain or encrypted.
//!
//! Before a connection is handed on, its HTTP request head is read into a
//! read-ahead buffer to tell WebSocket upgrades from page requests. Reads
//! return the buffered bytes first, so whoever takes the stream next, such as
//! the WebSocket handshake, still reads the whole request.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// An accepted connection's stream.
pub(crate) struct Stream {
    io: Io,
    /// Bytes read ahead and not yet returned by a read.
    ahead: Vec<u8>,
    /// How many bytes of `ahead` reads have returned.
    taken: usize,
}

enum Io {
    Plain(TcpStream),
    #[cfg(feature = "tls")]
    Tls(Box<tokio_rustls::server::TlsStream<TcpStream>>),
}

/// What the first byte of a connection showed.
#[cfg(feature = "tls")]
pub(crate) enum Secured {
    /// It began a TLS handshake, which completed.
    Tls(Stream),
    /// It began something else, probably plain HTTP.
    Plain(Stream),
    /// The peer closed the connection without sending anything.
    Closed,
}

impl Stream {
    /// Wraps an unencrypted connection.
    pub(crate) fn plain(stream: TcpStream) -> Self {
        Self::new(Io::Plain(stream))
    }

    fn new(io: Io) -> Self {
        Self {
            io,
            ahead: Vec::new(),
            taken: 0,
        }
    }

    /// The address of the peer.
    pub(crate) fn peer_addr(&self) -> io::Result<SocketAddr> {
        match &self.io {
            Io::Plain(stream) => stream.peer_addr(),
            #[cfg(feature = "tls")]
            Io::Tls(stream) => stream.get_ref().0.peer_addr(),
        }
    }

    /// The bytes read ahead that no read has returned yet.
    pub(crate) fn ahead(&self) -> &[u8] {
        &self.ahead[self.taken..]
    }

    /// Reads more bytes into the read-ahead buffer, which then holds at most
    /// `limit` bytes. Returns how many were added; `0` means the peer closed
    /// its side or the buffer is full.
    ///
    /// Dropping the returned future loses no bytes and leaves the buffer as
    /// it was.
    pub(crate) async fn read_ahead(&mut self, limit: usize) -> io::Result<usize> {
        use bytes::BufMut;

        self.ahead.drain(..self.taken);
        self.taken = 0;
        let room = limit.saturating_sub(self.ahead.len());
        if room == 0 {
            return Ok(0);
        }
        self.ahead.reserve(room);
        let mut buffer = (&mut self.ahead).limit(room);
        match &mut self.io {
            Io::Plain(stream) => stream.read_buf(&mut buffer).await,
            #[cfg(feature = "tls")]
            Io::Tls(stream) => stream.read_buf(&mut buffer).await,
        }
    }

    /// Completes the TLS handshake if the connection begins one.
    ///
    /// A TLS handshake begins with a handshake record, whose first byte is
    /// `0x16`; an HTTP request begins with a letter. The byte is peeked, so a
    /// plain connection is returned unread.
    #[cfg(feature = "tls")]
    pub(crate) async fn secure(self, acceptor: &tokio_rustls::TlsAcceptor) -> io::Result<Secured> {
        const HANDSHAKE_RECORD: u8 = 0x16;
        let Io::Plain(stream) = self.io else {
            return Ok(Secured::Tls(self));
        };
        debug_assert!(
            self.ahead.is_empty(),
            "nothing is read before the handshake"
        );
        let mut first = [0u8; 1];
        if stream.peek(&mut first).await? == 0 {
            return Ok(Secured::Closed);
        }
        if first[0] != HANDSHAKE_RECORD {
            return Ok(Secured::Plain(Self::plain(stream)));
        }
        let stream = acceptor.accept(stream).await?;
        Ok(Secured::Tls(Self::new(Io::Tls(Box::new(stream)))))
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.taken < this.ahead.len() {
            let ahead = &this.ahead[this.taken..];
            let size = ahead.len().min(buf.remaining());
            buf.put_slice(&ahead[..size]);
            this.taken += size;
            if this.taken == this.ahead.len() {
                // Free the buffer; the connection may stay open for hours.
                this.ahead = Vec::new();
                this.taken = 0;
            }
            return Poll::Ready(Ok(()));
        }
        match &mut this.io {
            Io::Plain(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(feature = "tls")]
            Io::Tls(stream) => Pin::new(stream.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().io {
            Io::Plain(stream) => Pin::new(stream).poll_write(cx, buf),
            #[cfg(feature = "tls")]
            Io::Tls(stream) => Pin::new(stream.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().io {
            Io::Plain(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
            #[cfg(feature = "tls")]
            Io::Tls(stream) => Pin::new(stream.as_mut()).poll_write_vectored(cx, bufs),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match &self.io {
            Io::Plain(stream) => stream.is_write_vectored(),
            #[cfg(feature = "tls")]
            Io::Tls(stream) => stream.is_write_vectored(),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().io {
            Io::Plain(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(feature = "tls")]
            Io::Tls(stream) => Pin::new(stream.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().io {
            Io::Plain(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(feature = "tls")]
            Io::Tls(stream) => Pin::new(stream.as_mut()).poll_shutdown(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncWriteExt;

    use super::*;

    #[tokio::test]
    async fn reads_return_the_bytes_read_ahead_first() {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let mut stream = Stream::plain(accepted);

        peer.write_all(b"hello").await.unwrap();
        while stream.ahead().len() < 4 {
            assert_ne!(stream.read_ahead(4).await.unwrap(), 0);
        }
        assert_eq!(stream.ahead(), b"hell");
        // A full buffer reads nothing more.
        assert_eq!(stream.read_ahead(4).await.unwrap(), 0);

        peer.write_all(b" world").await.unwrap();
        peer.shutdown().await.unwrap();
        let mut all = Vec::new();
        stream.read_to_end(&mut all).await.unwrap();
        assert_eq!(all, b"hello world");
        assert!(stream.ahead().is_empty());
    }
}
