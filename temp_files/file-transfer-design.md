# File transfer endpoints: agreed design

`temp_files/data-transport-discussion.md` describes an older design with a second WebSocket; it
was dropped in favour of the single-connection design below.

This file holds the agreed public API (Part A) and the internal design (Part B) of the
`Download<M>` / `Upload<M>` file transfer endpoints. Paths starting with `src/` mean
`crates/egui-states/src/`.

# Part A — Agreed design

## A1. Decisions and their reasons

| # | Decision | Reason |
|---|---|---|
| 1 | Transfers use the **existing WebSocket**, with file data in a separate low-priority lane. No second connection. | A second socket needs a ticket, a split accept path, a second heartbeat and reconnect logic, and gives no ordering between the two. Two TCP connections also still share the network, and the kernel then decides the split. On one connection the library decides: state always goes first. |
| 2 | A **credit window** bounds how much file data is in flight. | The receiver returns credit only after taking data off the socket, so at any moment at most one window of data sits between the two applications. A state message therefore waits behind at most `window / link rate` (about 2 ms for 256 KiB at 1 Gbit/s). This works on every platform, including the browser, without socket options. It also bounds memory. |
| 3 | Window and chunk size are **internal constants** (start with 256 KiB and 64 KiB), not user settings. | The benchmark should choose them. The other limits in `src/protocol/limits.rs` are not configurable either, and a setter can be added later without breaking anything. |
| 4 | Endpoints are **declared in the state tree**. | They get a name and id, typed metadata through the existing `Typed` machinery, generated Python/Rust bindings, and the layout-hash check. `Signal` is the precedent for a state that keeps no value. At runtime they stay out of the sync/ack lists. |
| 5 | **Two directional types**, `Download<M>` (server sends) and `Upload<M>` (client sends), named from the client's side. | Nearly every state in the library has a direction. Each handle has only the methods that work. A server that declares only downloads cannot be made to receive data, guaranteed by the layout and not by a missing callback. The names read backwards on the server (`download.send_file`), which the owner accepted. |
| 6 | **One unfinished transfer per endpoint.** A second send is refused (`Busy`). The server may opt into waiting with `wait=True`. | The owner wants no pile of queued transfers. The client stays simple: an endpoint is idle, has one offer, or has one transfer. Refusing can later be relaxed to a queue without breaking code; the reverse cannot. |
| 7 | Endpoints are independent, but **only one transfer sends data at a time per direction**. A second endpoint's transfer stays `Pending` until the first finishes. This includes browser downloads over HTTP. | Several transfers running together would either share the one credit window (no gain) or multiply the latency bound. Frames carry a transfer id, so interleaving can be added later. |
| 8 | **A transfer can carry several files**: one offer, one accept, one handle, one progress. | The owner's use case: several files into one folder with one path selection in the UI. Doing it in the library keeps name validation in one place and lets the browser deliver a set as one archive. |
| 9 | **Offer and accept**: nothing flows until the receiver accepts, and the receiver always chooses the destination. A name from the sender is only a hint. | The receiver can refuse by size or show a dialog first, and a sender can never choose a path on the other machine. |
| 10 | **The library validates file names** when an offer arrives (relative, no `..`, no absolute or drive parts). Names the receiver's OS cannot use, or names that collide case-insensitively, fail the accept before any data flows. No silent renaming. | Path traversal protection belongs in one place. Failing early is cheap because all names are in the offer. Silent renaming would give the user files that do not match what the sender listed. |
| 11 | **Writing is atomic per file**: temporary file in the target folder, rename on completion (replacing an existing file), original modification time applied. If a transfer fails, finished files stay and the partial one is removed. | A failed transfer never leaves a half file. Deleting tens of GB of finished files because the last one failed would be worse than a partial set. |
| 12 | **`Done` means the receiver has saved everything.** The receiver confirms at the end. | The sender must not report success before the data is on disk at its final name. |
| 13 | Client: **no call fails directly**; every failure shows as `Failed(..)` on the handle. Server: a problem **known at the call raises there**; everything later shows on the handle. | The client UI has to show failures anyway, and client calls like `Signal::set` are already infallible. On the server, `Busy`/`NotConnected`/missing file are known synchronously and an exception is the natural report. |
| 14 | **Browser downloads use plain HTTP** on the server's existing port, authorised by a one-time ticket sent over the WebSocket. | A page cannot write to disk. The save picker, private storage and service workers all need HTTPS or `localhost`, which a page like ExpertPI's (plain HTTP from another computer) does not have. The browser's download manager works everywhere and never touches wasm memory. Known cost: this path is outside the priority lane. |
| 15 | In the browser, several files arrive either as **one download per file** (`accept_download`) or as **one uncompressed zip** (`accept_download_bundled`, only used when there is more than one file). The client app chooses. | The owner rejected automatic archiving: unpacking tens of GB needs the disk space twice. Separate downloads lose folder structure and trigger a browser permission prompt, so the bundle exists for many small files. |
| 16 | **Browser uploads stay on the WebSocket.** | The browser reads the file in slices, so nothing is in memory as a whole, the priority lane is kept, and no cross-origin HTTP upload handling is needed. |
| 17 | Left out on purpose: public stream/sink layer, resume, integrity check, free-space check, a cargo feature for file handling, `send_dir`, Ctrl+C handling in waits, async `_with` variants on the download side, renaming/`root` on the client upload side. | Each was discussed and judged not needed for the first version. A stream of pieces can be done with repeated `send_bytes`. An app needing end-to-end integrity can put a hash in the metadata. File handling adds no dependency, so a feature flag would only add `cfg` combinations. |

## A2. Shared types

Defined once in the shared module (B1) and re-exported from the crate root for the client and
from `egui_states::server` for the Rust server.

```rust
pub trait TransferMeta: Typed + Serialize + DeserializeOwned + Send + Sync + 'static {}  // blanket impl; size <= VALUE_MAX_SIZE

pub struct TransferHandle { /* cheap Clone, Send + Sync */ }
impl TransferHandle {
    pub fn status(&self) -> TransferStatus;
    pub fn progress(&self) -> TransferProgress;
    pub fn name(&self) -> &str;
    pub fn is_finished(&self) -> bool;
    pub fn cancel(&self);                         // idempotent
    pub fn take_data(&self) -> Option<Vec<u8>>;   // accept_memory result, once, after Done; None for uploads
}

pub struct TransferProgress {
    pub done: u64,              // bytes, across all files
    pub total: Option<u64>,     // always Some in this version; optional so streams can be added later
    pub files_done: u32,
    pub files_total: u32,
}
impl TransferProgress { pub fn fraction(&self) -> Option<f32>; }

#[non_exhaustive]
pub enum TransferStatus { Pending, Running, Done, Failed(TransferError) }

#[non_exhaustive]
pub enum TransferError {
    Busy, NotConnected,                 // a refused send
    Disconnected,
    Rejected(String),
    Cancelled { by_peer: bool, reason: Option<String> },
    TooLarge, Unsupported(String), Meta(String), Io(String), Protocol(String),
    Timeout, InvalidInput(String),      // planner's addition: returned only by server calls (A6)
}
```

- Dropping a `TransferHandle` does not cancel the transfer.
- `Pending` covers: a `_with` picker still running, waiting for the peer's answer, and waiting
  behind a transfer of another endpoint.
- `name()` is empty while an upload's `_with` picker has not returned yet (store it in a
  `OnceLock<String>`).
- Reading `progress()` of a running transfer schedules the next repaint in 100 ms
  (`request_repaint_after`). A new offer or any status change requests one immediate repaint. An
  app that shows no progress gets no extra repaints.
- On disconnect every handle becomes `Failed(Disconnected)`. Nothing resumes after a reconnect.
  `last()` keeps returning that failed handle after the disconnect.

## A3. Client download API

```rust
#[derive(egui_states::State)]
pub struct AppState {
    pub results: Download<FileInfo>,   // typed metadata
    pub exports: Download,             // M = ()
}

impl<M: TransferMeta> Download<M> {
    pub fn take_offer(&self) -> Option<TransferOffer<M>>;   // call in update(); hands the offer over once
    pub fn last(&self) -> Option<TransferHandle>;           // most recent accepted transfer, finished or not
}

pub struct OfferedFile { /* name(): &str, size(): u64, modified(): Option<SystemTime> */ }

impl<M> TransferOffer<M> {              // Send, not Clone; dropping it rejects the offer
    pub fn name(&self) -> &str;              // the file name, or the name of the set
    pub fn files(&self) -> &[OfferedFile];   // one or more
    pub fn total_size(&self) -> Option<u64>;
    pub fn meta(&self) -> &M;
    pub fn is_cancelled(&self) -> bool;      // withdrawn by the server or lost to a disconnect

    #[cfg(not(target_arch = "wasm32"))]
    pub fn accept_file(self, path: impl Into<PathBuf>) -> TransferHandle;
    #[cfg(not(target_arch = "wasm32"))]
    pub fn accept_dir(self, dir: impl Into<PathBuf>) -> TransferHandle;
    #[cfg(not(target_arch = "wasm32"))]
    pub fn accept_file_with(self,
        pick: impl FnOnce(&TransferOffer<M>) -> Option<PathBuf> + Send + 'static) -> TransferHandle;
    #[cfg(not(target_arch = "wasm32"))]
    pub fn accept_dir_with(self,
        pick: impl FnOnce(&TransferOffer<M>) -> Option<PathBuf> + Send + 'static) -> TransferHandle;

    #[cfg(target_arch = "wasm32")]
    pub fn accept_download(self) -> TransferHandle;           // every file as its own browser download
    #[cfg(target_arch = "wasm32")]
    pub fn accept_download_bundled(self) -> TransferHandle;   // always exactly one browser download

    pub fn accept_memory(self, limit: u64) -> TransferHandle;
    pub fn reject(self, reason: impl Into<String>);
}
```

| Offer | `accept_file(_with)` | `accept_dir(_with)` | `accept_download` | `accept_download_bundled` | `accept_memory` |
|---|---|---|---|---|---|
| one file | yes | yes, under its own name | the file | the file, unchanged | yes, within `limit` |
| several files | no | yes | one download per file | one uncompressed zip | no |

Behaviour:

- `take_offer` returns the offer once. After that it returns `None` until the server sends the
  next one. The app owns the offer and may keep it across frames (for an egui-drawn dialog).
- A "no" in the table returns a handle already failed with `Unsupported`. The offer is used up
  and rejected, so the server's handle fails with the same reason. Nothing is written and `pick`
  is not run.
- Accepting a cancelled offer returns a handle that has already failed.
- Source and destination are independent: the client cannot tell `send_file` from `send_bytes`.
- `accept_dir` creates missing folders, including subfolders in names like `run42/0001.tiff`.
- `accept_memory` fails with `TooLarge` when the size exceeds `limit`.
- `_with`: returns a `Pending` handle at once and runs `pick` on a background (blocking-pool)
  thread of the client's runtime, never on its network threads. `Some(path)` accepts; `None`
  rejects as cancelled by the user. `last()` returns this handle while the dialog is open.
- Browser: the browser decides where the file goes. `accept_download` with several files flattens
  names (`run42/0001.tiff` becomes `run42_0001.tiff`) and the browser may ask once to allow
  several downloads. Progress for browser downloads is the server's view (bytes it has sent).

## A4. Client upload API

```rust
impl<M: TransferMeta> Upload<M> {
    pub fn send_file(&self, file: impl Into<LocalFile>, meta: M) -> TransferHandle;
    pub fn send_files(&self, files: Vec<LocalFile>, name: impl Into<String>, meta: M) -> TransferHandle;
    pub fn send_bytes(&self, data: Vec<u8>, name: impl Into<String>, meta: M) -> TransferHandle;

    #[cfg(not(target_arch = "wasm32"))]
    pub fn send_file_with(&self,
        pick: impl FnOnce() -> Option<LocalFile> + Send + 'static, meta: M) -> TransferHandle;
    #[cfg(not(target_arch = "wasm32"))]
    pub fn send_files_with(&self,
        pick: impl FnOnce() -> Option<Vec<LocalFile>> + Send + 'static,
        name: impl Into<String>, meta: M) -> TransferHandle;

    pub fn send_file_with_async<F>(&self, pick: F, meta: M) -> TransferHandle
        where F: Future<Output = Option<LocalFile>> + 'static;        // + Send on native
    pub fn send_files_with_async<F>(&self, pick: F, name: impl Into<String>, meta: M) -> TransferHandle
        where F: Future<Output = Option<Vec<LocalFile>>> + 'static;   // + Send on native

    pub fn last(&self) -> Option<TransferHandle>;
    pub fn is_busy(&self) -> bool;
}
```

- `LocalFile`: `From<PathBuf>`, `From<&Path>`, `From<&str>`, `From<String>` on native;
  `From<web_sys::File>` on wasm. A path in a wasm build is a compile error. No `rfd` dependency.
- Every call returns at once; opening and reading files happen in the background. A missing file
  shows as `Failed(Io)`.
- Each file is offered under its own file name; `send_files` names the set with `name`.
- A refused send (endpoint busy or not connected) returns a handle already failed with `Busy` or
  `NotConnected`. It does **not** replace `last()`.
- `_with` / `_with_async`: `Pending` handle at once; `Some` starts the upload; `None` ends the
  handle as cancelled and nothing is sent. The endpoint counts as busy while the picker is open.
  On native, also run the async `pick` on a background thread, so a blocking call inside it
  cannot stall the network threads.
- `_with` is the sync form (native only); `_with_async` is the async form (both targets). This
  naming is shared with the download side, which has only the sync form for now.

## A5. Python server API

Generated bindings (class pairs follow `Data` / `DataWithHeader`):

```python
self.files: s.Download = s.Download()  # Download
self.results: s.DownloadWithMeta[FileInfo] = s.DownloadWithMeta(3)  # Download<FileInfo>
self.logs: s.Upload = s.Upload()  # Upload
self.inputs: s.UploadWithMeta[InputInfo] = s.UploadWithMeta(4)  # Upload<InputInfo>
```

### Sending (`Download`)

```python
type PathLike = str | os.PathLike[str]


class Download:
    def send_file(
        self, path: PathLike, name: str | None = None, wait: bool = False, timeout: float | None = None
    ) -> TransferHandle: ...
    def send_files(
        self,
        paths: Sequence[PathLike],
        name: str,
        root: PathLike | None = None,
        wait: bool = False,
        timeout: float | None = None,
    ) -> TransferHandle: ...
    def send_bytes(
        self, data: Buffer, name: str, wait: bool = False, timeout: float | None = None
    ) -> TransferHandle: ...
    def last(self) -> TransferHandle | None: ...
    def connect(self, callback: Callable[[TransferHandle], Any]) -> None: ...  # a transfer finished
    def disconnect(self, callback: Callable[[TransferHandle], Any]) -> None: ...
    def disconnect_all(self) -> None: ...


class DownloadWithMeta[M]:
    # same methods; every send_* also takes a REQUIRED KEYWORD `meta: M`
    def send_file(
        self, path: PathLike, name: str | None = None, *, meta: M, wait: bool = False, timeout: float | None = None
    ) -> TransferHandle: ...
```

- `send_file`: the client sees the file's own name, or `name`.
- `send_files`: one transfer named `name`; each file under its own name, or with `root` its path
  relative to `root`.
- `send_bytes`: any buffer (`bytes`, `bytearray`, `memoryview`, NumPy array), copied once.
- There is no `send_dir`. A folder is sent with `send_files(paths, name, root=folder)`.
- Every call returns as soon as the offer is queued. Sizes and modification times are read at the
  call; the files are read later in Rust, outside the GIL, after the client accepts. A file that
  changed size by then fails the transfer.
- `wait=False` (default): raises `TransferError` (Busy) when the endpoint has an unfinished
  transfer. `wait=True`: blocks, GIL released, until the endpoint is free; `timeout` seconds, then
  `TimeoutError`. Waiting threads are served in call order. A disconnect during the wait raises
  `TransferError` (NotConnected). `timeout` without `wait=True` raises `ValueError`.
- Docs must warn: do not wait inside a signal callback. It holds a signal worker, and with one
  worker a wait for an upload would deadlock because `on_offer` needs a worker too.
- No client connected: `TransferError` (NotConnected). Nothing is kept for a later client.
- `connect(callback)`: called once per transfer of this endpoint when it is done or failed, with
  its handle, on the signal workers; exceptions go to the server's error handler.

### Receiving (`Upload`)

```python
class Upload:
    def on_offer(self, func: Callable[[TransferOffer], Any] | None) -> None: ...  # one function; None removes it
    def last(self) -> TransferHandle | None: ...
    def connect(self, callback: Callable[[TransferHandle], Any]) -> None: ...  # a transfer finished
    def disconnect(self, callback: Callable[[TransferHandle], Any]) -> None: ...
    def disconnect_all(self) -> None: ...


class UploadWithMeta[M]: ...  # same; offers and handles carry the metadata


class OfferedFile(NamedTuple):
    name: str
    size: int
    modified: float | None  # seconds since the epoch


class TransferOffer[M]:
    def name(self) -> str: ...
    def files(self) -> list[OfferedFile]: ...
    def total_size(self) -> int | None: ...
    def meta(self) -> M: ...  # None without metadata
    def accept_file(self, path: PathLike) -> TransferHandle: ...  # single file
    def accept_dir(self, dir: PathLike) -> TransferHandle: ...  # every file under its own name
    def accept_memory(self, limit: int) -> TransferHandle: ...  # single file
    def reject(self, reason: str = "") -> None: ...
```

- With no `on_offer` function set, every offer is rejected with "no receiver".
- The function runs on the signal workers. **An offer must be answered before the function
  returns**; if it returns without answering, or raises, the offer is rejected (an exception also
  goes to the error handler). Reason: Python has no reliable moment when an object is dropped.
- `accept_file` / `accept_memory` on a multi-file offer raise `ValueError` and leave the offer
  open. Answering twice raises `RuntimeError`.
- The announced sizes are enforced: a client that sends more than it announced fails the transfer
  (`Protocol`), so a size check in the callback can be trusted.
- Writing works as on the client (decision 11).

### Handle and types (both directions)

```python
class TransferHandle:
    def name(self) -> str: ...
    def status(self) -> TransferStatus: ...
    def error(self) -> TransferError | None: ...  # set when Failed
    def progress(self) -> TransferProgress: ...
    def is_finished(self) -> bool: ...
    def wait(self, timeout: float | None = None) -> None: ...  # GIL released
    def cancel(self, reason: str | None = None) -> None: ...  # idempotent; withdraws an unanswered offer
    def path(self) -> Path | None: ...  # received: the file, or the folder for accept_dir
    def take_data(self) -> bytes | None: ...  # accept_memory result, once, after Done
    def meta(self) -> Any: ...  # the transfer's metadata; None without metadata


class TransferStatus(Enum):  # member style as LogLevel
    Pending = 0
    Running = 1
    Done = 2
    Failed = 3


class TransferProgress(NamedTuple):
    done: int
    total: int | None
    files_done: int
    files_total: int


class TransferErrorKind(Enum):
    Busy = 0
    NotConnected = 1
    Rejected = 2
    Cancelled = 3
    Disconnected = 4
    Io = 5
    Unsupported = 6
    Protocol = 7
    TooLarge = 8


class TransferError(Exception):
    kind: TransferErrorKind  # str(e) is the message
```

- `wait` returns when done, raises the transfer's `TransferError` when it failed, and raises the
  builtin `TimeoutError` on timeout, leaving the transfer running.
- `send_*` raise `OSError` for a missing or unreadable file and `ValueError` for duplicate names,
  a file outside `root`, too many files, or metadata that is too large. So the Rust variants
  `Timeout`, `InvalidInput` and `Meta` have no `TransferErrorKind`: they become `TimeoutError`,
  `ValueError` / `OSError`, and `ValueError`.
- `TransferError` is the first custom exception in the package. Export it and the other new names
  from `egui_states/__init__.py`.

## A6. Native Rust server API (derived from Python by the planner; owner has not reviewed it)

Same methods and parameter order, positional as elsewhere in `src/server/` (compare
`DataTake::set(data, blocking, update, cache)`):

```rust
// egui_states::server
impl<M: TransferMeta> Download<M> {
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self>;
    pub fn send_file(&self, path: impl AsRef<Path>, name: Option<&str>, meta: &M,
                     wait: bool, timeout: Option<Duration>) -> Result<TransferHandle, TransferError>;
    pub fn send_files(&self, paths: &[PathBuf], name: &str, root: Option<&Path>, meta: &M,
                      wait: bool, timeout: Option<Duration>) -> Result<TransferHandle, TransferError>;
    pub fn send_bytes(&self, data: Vec<u8>, name: &str, meta: &M,
                      wait: bool, timeout: Option<Duration>) -> Result<TransferHandle, TransferError>;
    pub fn last(&self) -> Option<TransferHandle>;
    pub fn connect(&self, callback: impl Fn(TransferHandle) + Send + Sync + 'static) -> CallbackHandle;
}
impl<M: TransferMeta> Upload<M> {
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self>;
    pub fn on_offer(&self, func: impl Fn(TransferOffer<M>) + Send + Sync + 'static) -> CallbackHandle;
    pub fn last(&self) -> Option<TransferHandle>;
    pub fn connect(&self, callback: impl Fn(TransferHandle) + Send + Sync + 'static) -> CallbackHandle;
}
```

- The server `TransferHandle` is the same type as the client's plus server-only methods:
  `wait(timeout: Option<Duration>) -> Result<(), TransferError>`, `error()`, `path()`,
  `cancel_with(reason)`.
- A server-side `TransferOffer<M>` is an owned value whose drop rejects it, so unlike Python it
  may be moved out of the callback and answered later. Its accept methods mirror Python's and
  return `Result<TransferHandle, TransferError>` for misuse.
- `connect` / `on_offer` return a `CallbackHandle` like every other Rust server callback.

## A7. C ABI

The C ABI was removed from the library during this work, so no C support is planned.

---

# Part B — Internal design

## B1. Where the code goes

The word "transfer" is already used inside the library for chunked state messages
(`src/server_core/transfer.rs`, `TRANSFER_CHUNK_SIZE`, a `Chunk` type). Use `file_transfer` for
the new modules and `FILE_TRANSFER_*` for constants so nothing clashes. Public type names stay
as in Part A.

```
src/protocol/file_transfer.rs    wire types (always compiled, like the other protocol modules)
src/file_transfer/               engine shared by both peers; cfg(any(client, server, python))
    mod.rs      constants, TransferMeta, TransferError, TransferStatus, TransferProgress
    shared.rs   TransferShared: status, progress, error, result; what every handle wraps
    engine.rs   per-connection engine task: endpoint table, sender pump, receiver, credit
    names.rs    validation of offered names; platform checks; flattening for browser downloads
    source.rs   Source enum (bytes; files on native; web File on wasm)
    sink.rs     Sink enum (memory; file/dir on native)
    fs.rs       cfg(not(wasm32)): file reading and atomic writing
src/client/states/file_transfer.rs   Download<M>, Upload<M>, TransferOffer<M>, LocalFile, hub glue
src/client/net/                      wasm only: web File source, browser download trigger
src/server_core/file_transfer/       service, endpoint cores, send path with busy/wait, events, tickets
src/server_core/http/download.rs, zip.rs   HTTP route, streaming response, zip writer
src/server/file_transfer.rs          typed native Rust server API (A6)
src/python/pytransfer.rs             PyO3 free functions and private pyclasses
egui_states/transfer.py              public Python handle, offer, error and enum types
egui_states/structures.py            the four endpoint classes (generated code uses `s.Download`)
```

One engine for both peers, because each is sender in one direction and receiver in the other:
framing, credit accounting, the one-at-a-time rule, sources and sinks are the same code.

Constraints that follow from the build matrix, to respect from the first step:

- The shared engine and the client compile for wasm and for a native client-only build. There:
  no `tokio::select!`, `tokio::join!` or `#[tokio::test]` (tokio's `macros` feature is only on
  with `server` / `python`, and on wasm); use `futures_util::future::select` (it polls its first
  argument first, so it is biased) and build test runtimes by hand as the client tests do. No
  `tokio::spawn`, `spawn_blocking` or `tokio::time` in code that also builds for wasm.
- `Source` and `Sink` are enums with cfg-gated variants, not trait objects: async trait methods
  are not dyn-compatible, the crate has no `async-trait`, and the wasm file source is not `Send`.
- CI builds docs with default features and `-D warnings`: client rustdoc must not link to
  `server::` items. Expect `dead_code` across the clippy feature sets (for example the ticket
  frames on a native client); gate such items precisely instead of allowing the lint broadly.
- New dependencies need a `Cargo.lock` update (CI uses `--locked`).

## B2. Wire protocol

Append one variant to each header enum in `src/protocol/header.rs` (variant order is the wire
format; `ClientHeader::Handshake` must stay tag 4):

```rust
ServerHeader::Transfer(u64 /*endpoint id*/, TransferFrame, u32 /*payload len*/)   // tag 12
ClientHeader::Transfer(u64, TransferFrame, u32)                                   // tag 5
```

One frame enum serves both directions (`src/protocol/file_transfer.rs`):

```rust
pub(crate) enum TransferFrame {
    // sender -> receiver
    Offer  { serial: u32, meta_type: u32 },   // payload: postcard(OfferBody) then metadata bytes
    Chunk  { serial: u32, last: bool },       // DATA LANE. payload: raw bytes
    // receiver -> sender
    Accept { serial: u32, mode: AcceptMode }, // grants the initial window of credit
    Reject { serial: u32 },                   // payload: reason (UTF-8)
    Credit { serial: u32, bytes: u32 },
    // either side
    Cancel   { serial: u32 },                 // payload: reason
    Finished { serial: u32, error: Option<u8> }, // payload: message. The receiver confirms "saved"
                                                 // or reports failure. In HTTP modes the server sends it.
    // server -> client, HTTP download modes only
    Ticket       { serial: u32, file: u32, ticket: [u8; 16] },  // file index, u32::MAX = bundle
    HttpProgress { serial: u32, done: u64, files_done: u32 },
}
pub(crate) enum AcceptMode { Lane, Http, HttpBundled }
struct OfferBody { name: String, files: Vec<FileEntry> }   // FileEntry { name, size: u64, modified: Option<i64> (ms) }
```

- `serial` is allocated by the sender of the transfer, per endpoint, and identifies the transfer
  on the wire so late frames of an earlier transfer are ignored. It is **not** unique across
  reconnects (a restarted client starts again), so never use it as a long-lived key (see B7).
- The byte stream is the concatenation of the files in offer order; chunks are filled across file
  boundaries and carry no file index. Sizes are in the offer, so the receiver splits the stream.
  `last` marks the final chunk; a transfer with zero bytes sends one empty chunk with `last`.
- `meta_type` is the metadata type hash (0 for `()`), checked like `type_id` in value headers.
- Offer body and metadata are each limited to `VALUE_MAX_SIZE`; that bounds the file count
  (roughly tens of thousands). More is `ValueError` / `InvalidInput` / `Failed(Unsupported)`.
- Everything except `Chunk` travels in the **state lane**, so credit never waits behind data of
  the other direction.
- `Finished` cannot overtake data: the receiver produces it only after consuming the `last`
  chunk, and the sender reports `Done` only when it arrives. This also gives the documented
  ordering (state sent after a finished transfer arrives after the client saw `Done`).
- Tests and exhaustive matches to update: variant index arrays and byte assertions in
  `header.rs`; `payload_len` / `update_flag` / `incomplete` in `client_side.rs` and
  `server_side.rs`; `ClientMessage` in `server_core/net/socket_reader.rs`; `handle_message` and
  the `headers()` test helper in `client/messages.rs`.

Verified by the review: postcard encodes this nested enum (worst case about 43 bytes);
`MAX_SERVER_HEADER_LEN` only guards Data headers; "postcard body then metadata" parses with the
existing `take_from_bytes` helpers; a 2 MiB offer cannot push a merged client frame over
`WEBSOCKET_MAX_SIZE`.

## B3. Lanes and credit

**Constants**: `FILE_TRANSFER_WINDOW = 256 KiB`, `FILE_TRANSFER_CHUNK = 64 KiB`; credit is
returned in steps of about half a window to limit the number of credit frames. The benchmark in
step C3 may change these.

**Credit rules**

- The sender sends a chunk only while it holds credit for it.
- The receiver returns credit after its sink has taken the bytes. A slow disk therefore slows the
  sender.
- A chunk beyond the outstanding credit, or bytes beyond the announced total, is a protocol error
  that cancels that transfer. The receiver's internal queue is bounded by **bytes** (one window),
  not by message count, because chunks may be smaller than the nominal size.
- Only the head of the sender's queue of accepted transfers sends (A1 decision 7), so at most one
  window is in flight per direction.

**Server writer** (`src/server_core/net/session.rs`, `writer`): the `select!` is unbiased today.
Make it biased in the order **ping, state message, data chunk**. Ping must come first: with state
first, a state burst longer than the heartbeat timeout (a large `Data::set` queues all its chunks
at once) would send no ping and get a healthy client disconnected. The data branch is a bounded
channel receiver that the engine fills; it must await nothing but `recv()` (cancel safe), and it
must be disabled once `recv()` returns `None`, or the loop spins. Sending stays outside the
select. Chunks are never merged into state batches.

**Client send task** (`src/client/client.rs`, `send`): `futures_util::future::select` between
`MessagesSerializer::next()` (first, so it wins) and the data lane receiver.

**Lanes are per connection.** Create the data lane when a connection is established (server: in
`session::run` where `reader` / `writer` are spawned; client: in `run_session`), attach its
sending half to the engine, and detach on disconnect, which fails every transfer with
`Disconnected`. Transfers never survive a reconnect, so the lane does not need to travel through
`RunnerState` / `ChannelHolder` like `MessageReceiver` does. Also detach on the server's
failed-sync path.

**"Connected" for transfers means "lane attached"**, kept inside the service (or hub) mutex. The
check-and-register of a new transfer and the teardown must take that same lock. Otherwise a send
racing with a disconnect registers a transfer after teardown has run, its offer is drained by the
next connection, and the endpoint stays busy for ever. The global `connected` flag is set before
reader and writer exist, so do not use it for this.

**Readers never wait on the engine.** `session::reader` and `client::messages::handle_message`
hand a transfer frame to the engine with a non-blocking call. They must not await it.

**wasm note**: `ws_stream_wasm` has no backpressure (`send` completes at once). The credit window
is what stops the client from pushing a whole file into the browser's buffer.

Verified by the review: `SinkExt::send` flushes fully in tokio-tungstenite 0.30, so user space
holds at most the frame being sent; `TCP_NODELAY` is set on both sides; neither peer's batching
uses timers, so credit frames are not delayed. Tungstenite copies each frame once anyway, so do
not spend effort on a zero-copy chunk path.

## B4. Engine

- `TransferShared` (`shared.rs`): status, byte and file counters (atomics), error, optional
  result bytes, optional saved path, name (`OnceLock<String>`), and an optional repaint hook for
  the client. Every public handle type wraps an `Arc<TransferShared>`.
- Blocking waits (`handle.wait`, `send(wait=True)`) exist only on the server. Do not use
  `Event::wait_clear_deadline` for them: it consumes the flag, so with two waiters one sleeps for
  ever. Add a non-consuming wait to `src/event.rs` (or use a condition variable), gated to
  `server` / `python`.
- **Each peer has one engine task per connection, driven by a command channel.** This is needed
  on both sides: client handles are created before the client's runtime exists, and on the server
  `accept_*`, `cancel` and `send_*` are called from Python or callback worker threads that are
  outside the runtime, where `tokio::spawn` panics. Handle methods update shared state under the
  mutex and send a command; the engine task, started where the connection is established, does
  the spawning.
- Sender pump: takes the head of the accepted queue and loops: wait for credit, get the next
  chunk from the `Source`, push it into the data lane. After the last chunk it waits for
  `Finished`.
- Receiver: per accepted transfer, take chunks from the reader, write them to the `Sink`, return
  credit, and at the end finalize the sink and send `Finished`.
- Native file I/O uses `tokio::task::spawn_blocking` with `std::fs` (tokio's `fs` feature is not
  enabled and not needed), in blocks larger than one chunk (about 1 MiB) so disk calls stay few;
  credit still gates what is sent.
- A panic in the engine or pump task must not leave endpoints busy. Wrap these tasks the way the
  client wraps its connection tasks (`catch_panic` in `client.rs`) and fail all transfers.

## B5. Files, names, sinks (`names.rs`, `fs.rs`)

- Structural validation, both when a sender builds an offer and when a receiver gets one: UTF-8,
  components separated by `/` only, no empty / `.` / `..` component, no leading `/`, no
  backslash, no NUL or control characters, no exact duplicates. A received offer that fails is a
  protocol error and is never delivered to the app.
- Platform validation at accept, before any data flows (A1 decision 10): on Windows the invalid
  characters `<>:"|?*`, reserved device names, trailing dot or space; on Windows and macOS
  case-insensitive collisions. Failure: handle `Failed(Unsupported)` naming the file; the peer
  sees a rejection.
- File sink: temporary file `.<file name>.<unique>.part` in the target folder (unique from
  process id, a counter and the time, as `web.rs` does for its temp names; no random source is
  needed here). On completion set the modification time (`File::set_modified`) and `rename` over
  the target. Remove the temporary file on failure and cancel. An app exit or crash in the middle
  of a transfer can leave a `.part` file; that is accepted and should be documented.
- Multi-file source: sizes and modification times are read when the offer is built; a size that
  differs when the file is read fails the transfer.
- Browser flattening for `accept_download`: replace `/` by `_`; a collision after flattening
  fails the accept.

## B6. Client integration

- Create one `Arc<TransferHub>` in `StatesCreatorClient` (it already has the `MessageSender` and
  the `egui::Context`) and give it to every `Download` / `Upload` handle. `substate` builds a
  fresh creator and `get_values` discards the creator, so pass the hub down into sub-creators and
  return it inside `ValuesList`. `StatesCreatorBuild` constructs real handles without a runtime,
  so the hub needs a detached constructor for it.
- The hub holds the endpoint table and the command channel. `start_gui_client` starts the engine
  task per connection. The WebSocket URL (needed for browser download addresses) is only known in
  `build()`, because the host is set on the builder after `new`; hand it to the engine there.
- `ServerHeader::Transfer` is dispatched in `handle_message` to the hub, not through
  `ValuesList.states`. `ValuesList::disconnected()` must also fail the hub's transfers, under the
  same lock as registration (B3), while keeping `last()`.
- `_with` pickers on native: the engine task runs them with `spawn_blocking`. For `_with_async`
  on native use `spawn_blocking(|| Handle::current().block_on(fut))`. On wasm call
  `wasm_bindgen_futures::spawn_local` directly from the handle method.
- Gotchas:
  - Dropping a tokio runtime waits for running `spawn_blocking` closures, so an open picker
    dialog would keep the client thread alive after the last `Client` handle is dropped. Shut the
    runtime down with `shutdown_background`, or document the behaviour.
  - Tests assert exactly three `Client*` threads (`test_support::client_thread_count`). Blocking
    pool threads carry the runtime's thread name, so spawn blocking work only when a transfer or
    picker needs it. Verified: the tests are unaffected while nothing blocks at idle.
  - On wasm `web_sys::File` and the picker futures are not `Send`. If the state struct or the
    hub's channel needs `Send`, wrap these in a small wasm-only wrapper with an
    `unsafe impl Send` (the crate uses no wasm threads) and document why it is sound.
  - wasm has no timer in this crate. Progress repaints use `request_repaint_after`; timeouts for
    browser downloads live on the server.
  - Adding required methods to the public `StatesCreator` trait, and the derive macro matching
    field types by their last identifier (`crates/egui-states-macros/src/states.rs`), are source
    breaks: a user's own substate struct named `Download` or `Upload` would be read as the new
    kind. Say so in the PR description.

## B7. Server integration

- Register the kinds as `StateRef::Download` / `StateRef::Upload` in
  `src/server_core/server.rs`, with `sync_rank`, `sync` and `ack` returning nothing. Verified low
  friction: only `server.rs` (three matches) and `pyserver.rs` (`PyState`, `name`, `finalize`)
  match on `StateRef`. The id uniqueness check and `getters!` then cover the new kinds, and sync
  and acknowledgement code still never sees them.
- One transfer service object, owned by `Server` and cloned into `Session`, reachable from the
  Python `StateServerCore` and the native `StateServer` through `Server`. It owns the endpoint
  cores, the lane attachment, the command channel to the per-connection engine task, and the
  download tickets.
- Teardown: in `writer`'s cleanup (next to the `values.ack` reset) and on the failed-sync path,
  under the service lock: fail every transfer with `Disconnected`, detach the lane, abort HTTP
  download tasks, drop tickets.
- Send path: untyped (`meta: Bytes`), shared by Python and the Rust server. Under the lock: lane
  attached, endpoint free (or wait), then register. Input checks (stat files, names) happen
  before taking the slot. Waiters are served first come first served and wake on endpoint free,
  disconnect or timeout. The wait blocks a caller thread, never the runtime.
- **Events to callbacks: use each endpoint's own state id in `SignalsManager`**
  (`src/server_core/signals.rs`), in queue mode, with payload `(kind: u8, key: u64)` where kind
  is offer or finished and `key` is a server-local counter that is never reset. Reasons: this
  fits the existing per-id machinery unchanged (`signals_types[id]` in Python, `CallbackRegistry`
  and `CallbackHandle.value_id` in Rust), and callbacks of different endpoints run concurrently
  like those of other states. Two global reserved ids would serialize the callbacks of all
  endpoints behind one worker. What this requires:
  - `SignalsManager::reset()` clears pending events of ordinary ids on disconnect. Exempt the
    transfer endpoint ids there (as ids up to `MAX_RESERVED_ID` already are), so a completion
    event is never lost. Do not change `MAX_RESERVED_ID`; it would change every state id.
  - `set` silently drops an event for an id with no registered callback, and unregistering the
    last callback clears the queue. Every path that drops an offer event must reject that offer,
    or the client's upload stays `Pending` for ever. An upload offer on an endpoint without a
    receiver is rejected in the core ("no receiver") without an event.
  - Keep a finished transfer reachable by `key` only while its event is queued; if the event is
    dropped, drop the entry too.
  - In `pyserver.rs` `finalize`, keep the event payload type for these ids in `signals_types`,
    the way `Signal` and `Value` keep theirs, or `signal_get` fails to decode the event.
  - Events of one endpoint are served by one worker at a time. That is fine with one transfer
    per endpoint, but it means a finished callback runs only after `on_offer` has returned.

## B8. Python binding

- PyO3 here allows only **one** `#[pymethods]` block per class (`multiple-pymethods` is off).
  Keep thin wrapper methods in the existing block in `src/python/pyserver.rs` and put the bodies
  in `src/python/pytransfer.rs` as free functions, as `pydata.rs` and `pyimage.rs` do.
- Two **private** native classes wrap `Arc<TransferShared>` for the handle and the offer and
  return plain ints, tuples, strings and bytes. Verified: frozen pyclasses around an `Arc` are
  compatible with `gil_used = false`; register them in `src/python/mod.rs`.
- The **public** `TransferHandle` and `TransferOffer` are Python classes in
  `egui_states/transfer.py` that wrap the native ones and build `TransferStatus`,
  `TransferProgress`, `Path` and `TransferError`. Define `TransferError` in Python with
  `__init__(message, kind)`; either convert native error codes in the wrapper or raise it from
  Rust with `pyo3::import_exception!` (it imports lazily, so there is no cycle at module init).
- Avoid an import cycle: `transfer.py` must not import `structures.py`. Put the four endpoint
  classes in `structures.py` (generated code refers to them as `s.Download` through
  `import egui_states.structures as s`) and let `structures.py` import the types from
  `transfer.py`.
- `_initialize` in `egui_states/server.py` hands the signals manager only to `_SignalBase`, which
  also exposes the public `signal_set_to_queue` / `signal_set_to_single`. Add an intermediate
  base that carries only the signals manager, and use it for the endpoint classes.
- Metadata conversion: reuse `header_type`, `encode_single_header` and `decode_header` from
  `pyserver.rs` (make them `pub(crate)` or move them). Copy bytes out of any core lock before
  decoding, as their doc comment says.
- Blocking calls use `py.detach(..)` like `start` / `stop`. Convert Python arguments to Rust
  values before detaching.
- The `on_offer` dispatcher enforces "answered before return": after the user function returns or
  raises, it asks the core to reject the offer if it is still open.
- Update `egui_states/_core.pyi`, `egui_states/__init__.py`, and `src/build_scripts/python.rs` to
  emit `s.Download()` / `s.DownloadWithMeta(i)` / `s.Upload()` / `s.UploadWithMeta(i)` (the
  `SignalEmpty` versus `Signal` split is the pattern).

## B9. Browser downloads over HTTP

- Flow: the client sends `Accept { mode: Http | HttpBundled }`. The server creates a ticket (16
  random bytes; add `getrandom` as an optional dependency of the `server` / `python` features,
  the crate has no random source today) and sends `Ticket`. The client builds the download
  address from the host of its WebSocket URL and triggers the download. The server streams the
  file, reports `HttpProgress` (at most about ten times a second and at each file end), issues
  the next ticket in `Http` mode, and finally sends `Finished`.
- **Route: use a prefix outside `/ws`**, for example `/egui-states/transfer/<32 hex>`. The
  example `Trunk.toml` proxies `/ws` as a WebSocket-only backend, so a plain request below `/ws`
  would probably not pass it; add a second, plain HTTP proxy rule for the new prefix to the
  example and document it. The server ignores the path of WebSocket upgrades, so the two do not
  interfere. The route only shadows site files under the same prefix.
- A ticket is single use and bound to one transfer and file. If it is not fetched within about
  30 s (blocked download), the transfer fails. Unknown or used ticket: 404. Non-GET: 405 without
  consuming the ticket.
- Response headers: `200`, `Content-Type: application/octet-stream`, `Content-Length`,
  `Content-Disposition: attachment` with an ASCII `filename` and a UTF-8 `filename*`,
  `Cache-Control: no-store`, plus what `web.rs` already sends. No range support.
- Server changes (`src/server_core/net/transport.rs`, `session.rs`, `http/web.rs`):
  - `serve_web` peeks at the request head only when a web site is set; it must also peek when the
    tree has a `Download` endpoint. `Listener::serve_web` returns `Option<Self::Connection>` and
    the `pending` set is `JoinSet<Option<ClientReady<L>>>`: both need a third outcome for "run
    this long response". A boxed `'static` future keeps the run loop independent of the listener
    type; the in-memory test listener never yields it.
  - Run downloads in a separate task set, outside `pending` (capped at 32 and bound by the 10 s
    handshake deadline). Give the run loop a `join_next` branch for that set, or finished entries
    pile up. Abort the tasks on stop and on a transfer cancel.
  - Error responses (400, 404, 431) must work without a `WebSite`; `web::serve` needs one today
    and `RequestHead` is private to `web.rs`.
  - Use a longer body idle timeout than `WRITE_IDLE_TIMEOUT` (30 s): a browser set to ask where
    to save may stop reading while its dialog is open.
  - The origin policy and the token are only checked for WebSocket upgrades, so the ticket is the
    only access control. That is why it must be random, single use and short lived.
- Bundle (`zip.rs`): a streaming zip with stored entries, data descriptors, UTF-8 names, zip64
  records, relative names kept. All sizes are known, so compute `Content-Length` up front. CRC32
  via `crc32fast` (optional dependency of `server` / `python`). Name it `<set name>.zip`.
- wasm side: list the extra `web-sys` features directly (`Document`, `HtmlAnchorElement`,
  `HtmlElement`, `Element`, `Node`; for uploads `File`, `Blob`).
- **Unverified risk to test first**: some browsers have closed a page's WebSockets when a
  navigation turned into a download. If that happens here, every browser download would fail with
  `Disconnected`. Before building the rest of step C7, check in Chrome and Firefox that the
  chosen trigger (a link with the `download` attribute, or a hidden iframe) leaves the WebSocket
  open.

## B10. Browser uploads

Source for `LocalFile` on wasm: `File.slice(..)`, then `array_buffer()` through
`wasm_bindgen_futures::JsFuture`, copied into the chunk buffer. Read in blocks larger than a
chunk to keep promise round trips low. Sending Blob slices through the raw socket is a possible
later optimisation, not part of this plan.

## B11. State tree wiring (mechanical; mirror `DataTake<T, H>` and `Signal<T>`)

In dependency order:

1. Kind tags `DOWNLOAD_HASH_ID = 13`, `UPLOAD_HASH_ID = 14` in `src/client/states_creator.rs`
   (never reorder or reuse tags). Hash the kind alone for `M = ()` and kind plus the metadata
   type hash otherwise, following `data_type_hash`.
2. `StatesCreator::download` / `upload` in the trait, in `StatesCreatorClient`, and in
   `src/build_scripts/states_creator_build.rs` (same hash calls, so layout hashes match).
3. Derive macro arms in `crates/egui-states-macros/src/states.rs`.
4. `StateType::Download` / `Upload(String, Option<ObjectType>)` and `name()`; the arm in
   `get_all_enums_struct` (`src/build_scripts/scripts.rs`), without which the metadata's structs
   and enums are silently not emitted.
5. Generators: `python.rs` (`collect_object_types`, state line), `rust.rs` (`state_field_type`,
   `state_field_doc`, `state_initializer`).
6. Server core registration (B7), `PyState` variants and `finalize` arm, native Rust server
   handles and `StateServer::add_download` / `add_upload`, re-exports in `src/lib.rs` and
   `src/server/mod.rs`, the kind list in the crate docs.
7. Tests: a new struct in `crates/egui-states/tests/build_scripts_codegen.rs` using both kinds
   with and without metadata, a hash-match test and a new golden hash. Do not add to `AllKinds`
   (its golden value would change).
