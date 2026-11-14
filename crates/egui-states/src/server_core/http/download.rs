//! Browser downloads: the HTTP responses that stream a file transfer.
//!
//! The browser opens `DOWNLOAD_PATH` plus a one-time ticket that the server
//! sent over the WebSocket. The ticket is the only access control, since a
//! plain HTTP request carries neither the token nor an origin check: it is
//! random, works once, and expires if it is not fetched within seconds.

use std::fmt::Write as _;
use std::io;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::file_transfer::TransferError;
use crate::file_transfer::engine::{DOWNLOAD_PATH, Engine, HttpJob};
use crate::server_core::http::web::{HttpRequest, LINGER_TIMEOUT, MAX_REQUEST_HEAD};
use crate::server_core::http::zip::ZipLayout;

/// A browser that stops reading this long fails the download. It is longer
/// than for the web page: a browser set to ask where to save may wait for the
/// user's answer before it reads on.
const WRITE_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Bytes read and written at once.
const BLOCK: usize = 256 * 1024;

/// Whether `request` asks for a browser download.
pub(crate) fn is_download(request: &HttpRequest) -> bool {
    request
        .path()
        .is_some_and(|path| path.starts_with(DOWNLOAD_PATH))
}

/// Answers a request that is not a download when no web page is served.
pub(crate) async fn not_found(stream: TcpStream, request: HttpRequest) -> io::Result<()> {
    let status = request.error_status().unwrap_or(404);
    respond_error(stream, &request, status).await
}

/// Streams the browser download `request` asks for.
pub(crate) async fn serve(
    stream: TcpStream,
    request: HttpRequest,
    transfers: &Engine,
) -> io::Result<()> {
    if let Some(status) = request.error_status() {
        return respond_error(stream, &request, status).await;
    }
    // Only a GET claims the ticket; a HEAD or a probe must not use it up.
    if request.method() != Some("GET") {
        return respond_error(stream, &request, 405).await;
    }
    let ticket = request
        .path()
        .and_then(|path| path.strip_prefix(DOWNLOAD_PATH))
        .and_then(parse_ticket);
    let Some(job) = ticket.and_then(|ticket| transfers.claim_ticket(&ticket)) else {
        return respond_error(stream, &request, 404).await;
    };
    stream_job(stream, &request, job, transfers).await
}

fn parse_ticket(hex: &str) -> Option<[u8; 16]> {
    let hex = hex.split(['?', '#']).next()?;
    if hex.len() != 32 {
        return None;
    }
    let mut ticket = [0u8; 16];
    for (index, byte) in ticket.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(ticket)
}

async fn consume_head(stream: &mut TcpStream, request: &HttpRequest) -> io::Result<()> {
    if let Some(length) = request.head_len() {
        let mut buffer = vec![0; length.min(MAX_REQUEST_HEAD)];
        stream.read_exact(&mut buffer).await?;
    }
    Ok(())
}

async fn respond_error(
    mut stream: TcpStream,
    request: &HttpRequest,
    status: u16,
) -> io::Result<()> {
    consume_head(&mut stream, request).await?;
    let reason = match status {
        400 => "400 Bad Request",
        405 => "405 Method Not Allowed",
        431 => "431 Request Header Fields Too Large",
        _ => "404 Not Found",
    };
    let mut head = format!(
        "HTTP/1.1 {reason}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nContent-Length: 0\r\nCache-Control: no-store\r\n"
    );
    if status == 405 {
        head.push_str("Allow: GET\r\n");
    }
    head.push_str("\r\n");
    write_all(&mut stream, head.as_bytes()).await?;
    close(stream).await
}

async fn write_all(stream: &mut TcpStream, data: &[u8]) -> io::Result<()> {
    match tokio::time::timeout(WRITE_IDLE_TIMEOUT, stream.write_all(data)).await {
        Ok(result) => result,
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the browser stopped reading",
        )),
    }
}

async fn close(mut stream: TcpStream) -> io::Result<()> {
    stream.shutdown().await?;
    // Closing with unread input makes the kernel send a reset, which can
    // discard the end of the response before the browser reads it.
    let mut buffer = [0u8; 1024];
    let _ = tokio::time::timeout(LINGER_TIMEOUT, async {
        while stream.read(&mut buffer).await.is_ok_and(|size| size > 0) {}
    })
    .await;
    Ok(())
}

/// The `Content-Disposition` of a download saved as `name`: an ASCII
/// fallback for old browsers and the UTF-8 name for the others.
fn disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| match c {
            ' '..='~' if c != '"' && c != '\\' && c != '%' => c,
            _ => '_',
        })
        .collect();
    let mut encoded = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

/// Why streaming stopped.
enum Stop {
    /// The transfer was cancelled; the engine knows.
    Cancelled,
    /// The browser went away or stopped reading.
    Browser(io::Error),
    /// Reading the data failed.
    Source(String),
}

async fn stream_job(
    mut stream: TcpStream,
    request: &HttpRequest,
    mut job: HttpJob,
    transfers: &Engine,
) -> io::Result<()> {
    let (endpoint, serial) = (job.endpoint, job.serial);
    let result = async {
        consume_head(&mut stream, request).await.map_err(Stop::Browser)?;
        let layout = job.bundled.then(|| ZipLayout::new(&job.files));
        let length = match &layout {
            Some(layout) => layout.total_size(),
            None => job.files.iter().map(|file| file.size).sum(),
        };
        let content_type = if layout.is_some() {
            "application/zip"
        } else {
            "application/octet-stream"
        };
        let head = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\nContent-Disposition: {}\r\nCache-Control: no-store\r\n\r\n",
            disposition(&job.name)
        );
        write_all(&mut stream, head.as_bytes()).await.map_err(Stop::Browser)?;

        let mut done = job.offset;
        let mut crcs = Vec::with_capacity(job.files.len());
        for (index, file) in job.files.iter().enumerate() {
            if let Some(layout) = &layout {
                write_all(&mut stream, &layout.local_header(index))
                    .await
                    .map_err(Stop::Browser)?;
            }
            let mut crc = crc32fast::Hasher::new();
            let mut left = file.size;
            while left > 0 {
                let len = left.min(BLOCK as u64) as usize;
                let data = job.source.read(len).await.map_err(Stop::Source)?;
                crc.update(&data);
                write_all(&mut stream, &data).await.map_err(Stop::Browser)?;
                left -= len as u64;
                done += len as u64;
                if !transfers.http_progress(endpoint, serial, done) {
                    return Err(Stop::Cancelled);
                }
            }
            let crc = crc.finalize();
            crcs.push(crc);
            if let Some(layout) = &layout {
                write_all(&mut stream, &layout.descriptor(index, crc))
                    .await
                    .map_err(Stop::Browser)?;
            }
        }
        if let Some(layout) = &layout {
            write_all(&mut stream, &layout.central_directory(&crcs))
                .await
                .map_err(Stop::Browser)?;
        }
        if !transfers.http_progress(endpoint, serial, done) {
            return Err(Stop::Cancelled);
        }
        Ok(())
    }
    .await;

    match result {
        Ok(()) => {
            let closed = close(stream).await;
            transfers.http_finished(job);
            closed
        }
        Err(Stop::Cancelled) => Ok(()),
        Err(Stop::Browser(error)) => {
            transfers.http_cancelled(endpoint, serial, "the browser stopped the download");
            Err(error)
        }
        Err(Stop::Source(error)) => {
            transfers.http_failed(endpoint, serial, TransferError::Io(error));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tickets_parse_from_hex() {
        let hex = "00112233445566778899aabbccddeeff";
        assert_eq!(
            parse_ticket(hex).unwrap(),
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff
            ]
        );
        assert!(parse_ticket(&format!("{hex}?x=1")).is_some());
        assert!(parse_ticket("0011").is_none());
        assert!(parse_ticket(&hex.replace('0', "g")).is_none());
    }

    #[test]
    fn dispositions_keep_utf8_names() {
        assert_eq!(
            disposition("run 42.zip"),
            "attachment; filename=\"run 42.zip\"; filename*=UTF-8''run%2042.zip"
        );
        assert_eq!(
            disposition("ünï\"x.bin"),
            "attachment; filename=\"_n__x.bin\"; filename*=UTF-8''%C3%BCn%C3%AF%22x.bin"
        );
    }
}
