//! Optional static web page served on the same port as the WebSocket.
//!
//! When a page is configured, every new connection is peeked (not read) until
//! its HTTP request head is complete. WebSocket upgrades get the untouched
//! `TcpStream` back, so the WebSocket path keeps its exact stream type and no
//! wrapper or extra copy is involved. Other requests are answered from an
//! in-memory snapshot of the page directory.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use parking_lot::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Shared slot holding the currently served page, read once per connection.
pub(crate) type WebSlot = Arc<RwLock<Option<Arc<WebSite>>>>;

// Large enough for browser requests carrying cookies of other localhost apps.
const MAX_REQUEST_HEAD: usize = 16 * 1024;
const MAX_HEADERS: usize = 64;
const MAX_DIRECTORY_DEPTH: usize = 32;
const PARTIAL_HEAD_RETRY: Duration = Duration::from_millis(5);
const LINGER_TIMEOUT: Duration = Duration::from_secs(1);
// A slow link still makes progress; a peer that accepts nothing for this long
// has stalled and would otherwise hold its socket and page snapshot forever.
const WRITE_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

struct WebFile {
    body: Bytes,
    content_type: &'static str,
    etag: String,
}

/// In-memory snapshot of a static web page directory.
pub(crate) struct WebSite {
    root: PathBuf,
    files: HashMap<String, WebFile>,
}

impl WebSite {
    /// Loads every file below `root`, which must contain `index.html`.
    pub(crate) fn load(root: &Path) -> Result<Self, String> {
        let root = std::path::absolute(root)
            .map_err(|e| format!("invalid web page path {}: {e}", root.display()))?;
        if !root.is_dir() {
            return Err(format!(
                "web page path {} is not a directory",
                root.display()
            ));
        }

        let mut files = HashMap::new();
        load_directory(&root, "", 0, &mut files)?;
        if !files.contains_key("index.html") {
            return Err(format!(
                "web page directory {} does not contain index.html",
                root.display()
            ));
        }

        Ok(Self { root, files })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    fn get(&self, path: &str) -> Option<&WebFile> {
        self.files.get(path)
    }
}

fn load_directory(
    directory: &Path,
    prefix: &str,
    depth: usize,
    files: &mut HashMap<String, WebFile>,
) -> Result<(), String> {
    if depth > MAX_DIRECTORY_DEPTH {
        return Err(format!(
            "web page directory {} is nested too deeply",
            directory.display()
        ));
    }

    let entries = std::fs::read_dir(directory)
        .map_err(|e| format!("reading directory {} failed: {e}", directory.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|e| format!("reading directory {} failed: {e}", directory.display()))?;
        let path = entry.path();
        // Names that are not UTF-8 cannot be addressed by a request path.
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let key = format!("{prefix}{name}");
        // Follows symlinks, so linked files and directories are served too.
        let metadata = std::fs::metadata(&path)
            .map_err(|e| format!("reading {} failed: {e}", path.display()))?;

        if metadata.is_dir() {
            load_directory(&path, &format!("{key}/"), depth + 1, files)?;
        } else if metadata.is_file() {
            let body = std::fs::read(&path)
                .map_err(|e| format!("reading {} failed: {e}", path.display()))?;
            // Like common web servers, tag files by modification time and size,
            // which avoids hashing large WASM binaries.
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |time| time.as_nanos());
            let etag = format!("\"{modified:x}-{:x}\"", body.len());
            files.insert(
                key,
                WebFile {
                    content_type: content_type(&name),
                    body: Bytes::from(body),
                    etag,
                },
            );
        }
    }
    Ok(())
}

fn content_type(name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map_or("", |(_, ext)| ext);
    match extension.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

struct RequestHead {
    length: usize,
    method: String,
    path: String,
    if_none_match: Option<String>,
}

enum Peeked {
    /// The peer closed the connection before sending a request.
    Closed,
    WebSocket,
    Http(RequestHead),
    Invalid,
    TooLarge,
}

/// Outcome of inspecting a new connection while a web page is configured.
pub(crate) enum Dispatch {
    /// A WebSocket upgrade request; the stream is unread.
    WebSocket(TcpStream),
    /// A plain HTTP request; answer it with [`serve`].
    Http(TcpStream, HttpRequest),
    /// The peer closed the connection without sending anything.
    Closed,
}

/// Parsed request head, or the error status to answer with.
pub(crate) struct HttpRequest(Result<RequestHead, u16>);

/// Peeks at the request head to tell WebSocket upgrades from HTTP requests.
///
/// The caller bounds this with a timeout; a peer that never completes its
/// request head would otherwise keep the connection open.
pub(crate) async fn dispatch(stream: TcpStream) -> io::Result<Dispatch> {
    let mut buffer = vec![0; MAX_REQUEST_HEAD];
    Ok(match peek_request(&stream, &mut buffer).await? {
        Peeked::Closed => Dispatch::Closed,
        Peeked::WebSocket => Dispatch::WebSocket(stream),
        Peeked::Http(head) => Dispatch::Http(stream, HttpRequest(Ok(head))),
        Peeked::Invalid => Dispatch::Http(stream, HttpRequest(Err(400))),
        Peeked::TooLarge => Dispatch::Http(stream, HttpRequest(Err(431))),
    })
}

async fn peek_request(stream: &TcpStream, buffer: &mut [u8]) -> io::Result<Peeked> {
    let mut previous = 0;
    loop {
        let size = stream.peek(buffer).await?;
        if size == 0 {
            return Ok(Peeked::Closed);
        }

        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut request = httparse::Request::new(&mut headers);
        match request.parse(&buffer[..size]) {
            Ok(httparse::Status::Complete(length)) => {
                let header = |name: &str| {
                    request
                        .headers
                        .iter()
                        .find(|h| h.name.eq_ignore_ascii_case(name))
                        .and_then(|h| std::str::from_utf8(h.value).ok())
                };
                let upgrade = header("Upgrade").is_some_and(|value| {
                    value
                        .split(',')
                        .any(|p| p.trim().eq_ignore_ascii_case("websocket"))
                });
                if upgrade {
                    return Ok(Peeked::WebSocket);
                }
                return Ok(Peeked::Http(RequestHead {
                    length,
                    method: request.method.unwrap_or_default().to_owned(),
                    path: request.path.unwrap_or_default().to_owned(),
                    if_none_match: header("If-None-Match").map(str::to_owned),
                }));
            }
            Ok(httparse::Status::Partial) if size == buffer.len() => {
                return Ok(Peeked::TooLarge);
            }
            Ok(httparse::Status::Partial) => {
                // Peeking leaves the bytes queued, so the socket stays readable
                // and another peek would return at once. Back off until more
                // of the head has arrived.
                if size == previous {
                    tokio::time::sleep(PARTIAL_HEAD_RETRY).await;
                }
                previous = size;
            }
            Err(_) => return Ok(Peeked::Invalid),
        }
    }
}

/// Answers one HTTP request from `site` and closes the connection.
pub(crate) async fn serve(
    stream: TcpStream,
    request: HttpRequest,
    site: &WebSite,
) -> io::Result<()> {
    serve_with_idle_timeout(stream, request, site, WRITE_IDLE_TIMEOUT).await
}

async fn serve_with_idle_timeout(
    mut stream: TcpStream,
    request: HttpRequest,
    site: &WebSite,
    idle_timeout: Duration,
) -> io::Result<()> {
    let mut buffer = vec![0; MAX_REQUEST_HEAD];
    let response = match request.0 {
        Ok(head) => {
            // Consume the peeked head; it is already queued, so this does not wait.
            stream.read_exact(&mut buffer[..head.length]).await?;
            respond(&head, site)
        }
        Err(status) => Response::error(status),
    };

    let mut header = format!(
        "HTTP/1.1 {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n",
        status_line(response.status),
    );
    // A 304 omits the length rather than misstate that of the cached file.
    if let Some(length) = response.content_length {
        let _ = write!(header, "Content-Length: {length}\r\n");
    }
    if let Some(file) = response.file {
        let _ = write!(
            header,
            "Content-Type: {}\r\nCache-Control: no-cache\r\nETag: {}\r\n",
            file.content_type, file.etag
        );
    } else {
        header.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    }
    if response.status == 405 {
        header.push_str("Allow: GET, HEAD\r\n");
    }
    header.push_str("\r\n");

    write_all_with_idle_timeout(&mut stream, header.as_bytes(), idle_timeout).await?;
    if let Some(body) = response.body {
        write_all_with_idle_timeout(&mut stream, body, idle_timeout).await?;
    }
    stream.shutdown().await?;

    // Closing a socket with unread input makes the kernel send a reset, which
    // can discard the response before the browser reads it. Drain whatever the
    // peer still sends until it closes its side.
    let _ = tokio::time::timeout(LINGER_TIMEOUT, async {
        while stream.read(&mut buffer).await.is_ok_and(|size| size > 0) {}
    })
    .await;
    Ok(())
}

/// Writes all of `data`, failing once the peer accepts nothing for `idle_timeout`.
async fn write_all_with_idle_timeout(
    stream: &mut TcpStream,
    mut data: &[u8],
    idle_timeout: Duration,
) -> io::Result<()> {
    while !data.is_empty() {
        let written = tokio::time::timeout(idle_timeout, stream.write(data))
            .await
            .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        data = &data[written..];
    }
    Ok(())
}

struct Response<'a> {
    status: u16,
    content_length: Option<usize>,
    file: Option<&'a WebFile>,
    body: Option<&'a [u8]>,
}

impl Response<'static> {
    fn error(status: u16) -> Self {
        let body = status_line(status).as_bytes();
        Self {
            status,
            content_length: Some(body.len()),
            file: None,
            body: Some(body),
        }
    }
}

fn respond<'a>(head: &RequestHead, site: &'a WebSite) -> Response<'a> {
    let is_head = match head.method.as_str() {
        "GET" => false,
        "HEAD" => true,
        _ => return Response::error(405),
    };
    let Some(file) = resolve_path(&head.path).and_then(|path| site.get(&path)) else {
        return Response::error(404);
    };

    if head
        .if_none_match
        .as_deref()
        .is_some_and(|tags| tags.split(',').any(|t| t.trim() == file.etag))
    {
        return Response {
            status: 304,
            content_length: None,
            file: Some(file),
            body: None,
        };
    }

    Response {
        status: 200,
        content_length: Some(file.body.len()),
        file: Some(file),
        body: (!is_head).then_some(&file.body[..]),
    }
}

/// Maps a request target to a key of the in-memory file table.
///
/// Only loaded files can be returned, so `..` segments cannot escape the page
/// directory; they simply match nothing.
fn resolve_path(target: &str) -> Option<String> {
    let path = target.split(['?', '#']).next()?;
    let path = path.strip_prefix('/')?;
    let mut decoded = percent_decode(path)?;
    if decoded.is_empty() || decoded.ends_with('/') {
        decoded.push_str("index.html");
    }
    Some(decoded)
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            output.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).ok()
}

fn status_line(status: u16) -> &'static str {
    match status {
        200 => "200 OK",
        304 => "304 Not Modified",
        400 => "400 Bad Request",
        404 => "404 Not Found",
        405 => "405 Method Not Allowed",
        431 => "431 Request Header Fields Too Large",
        _ => "500 Internal Server Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(files: &[(&str, &str)]) -> (tempdir::TempDir, WebSite) {
        let dir = tempdir::TempDir::new();
        for (name, content) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        let site = WebSite::load(dir.path()).unwrap();
        (dir, site)
    }

    // Minimal self-cleaning temporary directory; the crate has no dev-dependencies.
    mod tempdir {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicUsize, Ordering};

        pub(super) struct TempDir(PathBuf);

        impl TempDir {
            pub(super) fn new() -> Self {
                static COUNTER: AtomicUsize = AtomicUsize::new(0);
                let path = std::env::temp_dir().join(format!(
                    "egui-states-web-{}-{}",
                    std::process::id(),
                    COUNTER.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }

            pub(super) fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn head(method: &str, path: &str) -> RequestHead {
        RequestHead {
            length: 0,
            method: method.into(),
            path: path.into(),
            if_none_match: None,
        }
    }

    #[test]
    fn load_requires_index_and_collects_nested_files() {
        let dir = tempdir::TempDir::new();
        std::fs::write(dir.path().join("app.js"), "js").unwrap();
        let error = WebSite::load(dir.path()).err().unwrap();
        assert!(error.contains("index.html"), "{error}");

        let missing = dir.path().join("missing");
        let error = WebSite::load(&missing).err().unwrap();
        assert!(error.contains("not a directory"), "{error}");

        let (_dir, site) = site(&[("index.html", "page"), ("assets/app.wasm", "wasm")]);
        assert_eq!(&site.get("index.html").unwrap().body[..], b"page");
        let wasm = site.get("assets/app.wasm").unwrap();
        assert_eq!(wasm.content_type, "application/wasm");
        assert!(wasm.etag.starts_with('"') && wasm.etag.ends_with('"'));
    }

    #[test]
    fn request_paths_resolve_to_loaded_files_only() {
        let (_dir, site) = site(&[
            ("index.html", "root"),
            ("sub/index.html", "sub"),
            ("a b.js", "x"),
        ]);

        for (target, expected) in [
            ("/", Some("root")),
            ("/index.html?v=1", Some("root")),
            ("/sub/", Some("sub")),
            ("/a%20b.js", Some("x")),
            ("/../index.html", None),
            ("/sub", None),
            ("relative", None),
            ("/bad%zz", None),
        ] {
            let response = respond(&head("GET", target), &site);
            match expected {
                Some(body) => {
                    assert_eq!(response.status, 200, "{target}");
                    assert_eq!(response.body.unwrap(), body.as_bytes(), "{target}");
                }
                None => assert_eq!(response.status, 404, "{target}"),
            }
        }
    }

    #[tokio::test]
    async fn a_peer_that_stops_reading_releases_the_connection() {
        // Large enough to fill the socket buffers of both peers.
        let page = "x".repeat(64 * 1024 * 1024);
        let (_dir, site) = site(&[("index.html", &page)]);
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        peer.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
        let (stream, _) = listener.accept().await.unwrap();

        let Dispatch::Http(stream, request) = dispatch(stream).await.unwrap() else {
            panic!("expected an HTTP request");
        };
        let served = tokio::time::timeout(
            Duration::from_secs(5),
            serve_with_idle_timeout(stream, request, &site, Duration::from_millis(200)),
        )
        .await
        .expect("a stalled download must not block the server");
        assert_eq!(served.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn methods_and_conditional_requests() {
        let (_dir, site) = site(&[("index.html", "root")]);

        assert_eq!(respond(&head("POST", "/"), &site).status, 405);

        let response = respond(&head("HEAD", "/"), &site);
        assert_eq!((response.status, response.content_length), (200, Some(4)));
        assert!(response.body.is_none());

        let etag = site.get("index.html").unwrap().etag.clone();
        let mut conditional = head("GET", "/");
        conditional.if_none_match = Some(format!("\"other\", {etag}"));
        let response = respond(&conditional, &site);
        assert_eq!(response.status, 304);
        assert!(response.body.is_none());
    }
}
