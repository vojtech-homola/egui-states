# Architecture

`egui-states` derives a synchronized state tree from Rust types and exposes the
same tree to an egui client and either a Rust or Python server. Applications use
typed state handles; the library owns the WebSocket messages, serialization,
and synchronization bookkeeping.

## From a state type to two peers

1. A Rust struct derives `State`. Its field names become path segments below
   `root`, and each state handle contributes its kind and value type to a stable
   layout hash.
2. `ClientBuilder` walks that type to create client handles and the message
   dispatch table.
3. `generate_python` or `generate_rust` walks the same state definition from a
   build script or a generator executable and writes matching server bindings.
4. The peers always compare the wire-protocol version during the WebSocket
   handshake. They can also compare an application version and token when those
   checks are configured. Before that, the server refuses the WebSocket upgrade
   of a browser page whose `Origin` it does not allow; native clients send no
   `Origin`.

Generated bindings should not be edited. Their contents and layout hash are
derived from the shared Rust state definition. The layout hash is not enforced by
default; applications can pass `ClientBuilder::get_version_hash()` as the
client application version and the generated `StatesServer::VERSION_HASH` as
the server version to reject mismatched state trees during the handshake.

The state definition can be imported from a library crate or compiled from the
same source module into both the GUI and build-script crates. See the
[counter build script](../example/counter/gui/build.rs) for a shared source
module and the [showcase build script](../example/showcase/gui/build.rs) for
an application library imported by a separate GUI launcher package.

## State directions

- `Value` is stored by both peers. Either peer can replace it; signaling an
  update also schedules registered server callbacks. When both write at the
  same time, the server wins; see
  [Concurrent `Value` writes](#concurrent-value-writes).
- `Static`, collections, `Data`, and `Image` are controlled by the server and
  read by the client.
- `Signal` carries an event from the client to server callbacks without
  retaining a value. A Rust server may also emit it directly to its local
  callbacks.
- `ValueTake` and `DataTake` carry a server-to-client value that the client
  consumes once. `DataMultiTake` maintains the same lifecycle independently
  for each numeric key.
- `Download` sends files from the server to the client and `Upload` from the
  client to the server; see [File transfers](#file-transfers). They hold no
  value and are never synchronized.

## Concurrent `Value` writes

The server is authoritative. When the server program and the user change the
same `Value` or `ValueAtomic` at the same time, the server's write wins: the
program driving the server takes precedence over a human editing the UI.

Each server write stays pending until the client acknowledges it, and so does
the synchronization sent when a client connects. A client write that reaches
the server while a server write is still pending was made before the client
applied that server value, so the server discards it. The stored value stays
unchanged, no callback runs (not even for `set_signal`), and neither peer
reports anything. The client then receives the pending server value and
replaces its local copy, so both peers end with the server's value and the
user's edit is lost. Because the synchronization on connect is pending as well,
edits made before the client received the server's state cannot overwrite it.

A client write is therefore discarded when it arrives within about one network
round trip after a server write. A server that sets a `Value` continuously, for
example a progress indicator or a live measurement, leaves the user little or
no chance to change it, especially over a slower network. Store values that the
server streams in a `Static`, and send user requests to change them as a
`Signal`, which the server never discards; the server callback then decides
whether to apply the request.

## Callback delivery modes

Signals and signaled `Value` changes use one of two server-side modes:

- Single mode (`NoQueue`, the default) coalesces pending changes for a state to
  the latest value while a callback for that state is outstanding.
- Queue mode (`Queue`) preserves every change and delivers them in order.

Callbacks for different states may run concurrently when the server has more
than one signal worker. A callback handle owns its registration; dropping the
handle disconnects it. In the native Rust server, neither state handles nor
callback handles keep the server alive, so callbacks may capture state handles
without forming a cycle; the server stops when its last `StateServer` is
dropped.

`connect_previous` receives the value last delivered to that callback. In
single mode this is not necessarily the immediately preceding network update:
if `a -> b -> c` coalesces to `c`, the callback receives `(c, a)`.

## Transfers and acknowledgements

Large numeric buffers, image sets and collection replacements are split into
chunks of 512 KiB and published to a client handle only after the last chunk
arrives. Small messages queued together are merged into one frame of up to
10 MB, but a chunk is never merged: the WebSocket library moves the unsent rest
of a message after every partial socket write, so large frames cost time
quadratic in their size. Acknowledgements provide backpressure and release
pending server work.

When a client connects, every state is synchronized from the server's current
contents; a change made while the client was away is not replayed. A
`DataMulti`, `DataMultiTake` or `ImageMulti` is reset first and then receives
its keys one by one, so for a moment a reconnected client shows the collection
empty or with only some keys, until the last key has arrived. A partial transfer from a lost
connection is dropped on both sides.

A whole-collection replacement of a `VecState` or `MapState` larger than one
message is split at item boundaries and applied on the client only after its
last part arrives. These parts need no acknowledgement.

All keys in an `ImageMulti` share one ordered transfer stream. Sets and updates
that require multiple messages finish before a later remove or reset is sent;
controls do not cancel preceding chunks. A disconnect invalidates pending work
with a transfer generation, and reconnect synchronization resets the client
collection before sending the server's current keys. Each synchronized image
must be acknowledged before subsequently queued work is released.

Waiting for acknowledgements slows the server down to the client's pace. A
client that disappears without closing the connection would make it wait
forever, so the server pings the client every heartbeat interval and treats a
client that sent no frame (an acknowledgement, a value, or the automatic pong)
for the heartbeat timeout as disconnected. The connection is then closed, also
when a send is waiting for socket buffer space, and pending work is released as
for any disconnect. Native clients answer pings on their network task and
browsers answer them outside the page, so a busy UI does not count as silence.
A client receiving a large message sends nothing until it arrives, so the
timeout must exceed the longest transfer over the slowest expected network.

For one-shot transfers, `blocking = true` prevents a later send from overtaking
the pending transfer. Calling the client handle's `take` method consumes the
value and sends the acknowledgement. For numeric takes, `cache = true` also
retains the last transfer for synchronization with a newly connected client.
A reconnected `DataMultiTake` holds only the cached keys: values the client
had not taken before the disconnect are dropped.

## File transfers

`Download<M>` and `Upload<M>` move files and buffers that are not state. They
share the WebSocket with the state messages but not its queue.

**Two lanes.** Each peer's writer sends a heartbeat ping first, then any
queued state message, and a chunk of file data (64 KiB) only when no state
message waits. The control frames of a transfer (offer, accept, credit,
cancel, finished) travel with the state messages and keep their order, so a
state change the server sends after a finished transfer arrives after the
client sees it done.

**Credit.** The receiver grants a window of 256 KiB and returns credit as its
sink takes the data. The sender never has more than one window between the
two applications, wherever the bytes sit: its socket buffer, the network or
the receiver's queue. A state message therefore waits behind at most about
`window / link rate` of file data, about 2 ms at 1 Gbit/s, and a slow disk on
the receiver slows the sender instead of filling memory. The window also caps
throughput at about one window per round trip, which matters only over long
round trips.

**Offers.** Nothing flows until the receiver accepts, and the receiver
chooses the destination; a name from the sender is only a hint. Names are
checked when an offer arrives (relative, `/` between folders, no `.` or `..`)
and, before any data flows, against the receiver's system (Windows reserved
names and characters, names that differ only in letter case). Each file is
written to a temporary file and renamed over its target when complete, with
the sender's modification time. When a transfer fails, the partial file is
removed and complete files of the set stay.

**One at a time.** An endpoint carries at most one unfinished transfer: a
second send fails with `Busy` or, on the server with `wait`, waits; waiting
senders are served in call order. Endpoints are independent, but only one
transfer per direction sends data at a time, so a transfer accepted on a
second endpoint stays pending until the first finishes. A server endpoint
without a receiver rejects uploads with "no receiver".

**Disconnects.** A transfer belongs to one connection. A disconnect fails every
transfer on both sides as disconnected, and nothing is resumed after a
reconnect. A client process that exits during a transfer can leave a
temporary `.part` file behind.

**Browsers.** A page cannot write to disk, and the browser's file APIs need
HTTPS or `localhost`. A browser client therefore accepts a download as browser
downloads: the server sends a one-time ticket over the WebSocket, and the
browser fetches `/egui-states/transfer/<ticket>` from the server's port over
plain HTTP, one download per file (folders flattened into the name) or one
uncompressed zip of all files. The ticket is the only access control, so it is
random, works once and expires after 30 seconds. Progress is the server's view
of the bytes it has sent. These downloads run on their own TCP connection,
outside the credit window, so they share the link with the state messages
like any other download. A development server that serves the page must
forward this path too; the showcase's `Trunk.toml` shows how. Browser uploads
read `File` objects in slices and travel in the lane like native ones.

## Data headers

A `Data`, `DataMulti`, `DataTake` or `DataMultiTake` can carry a typed header
next to its buffer: any `Typed` value up to the size of a value (1 MB), for
example the shape, the axis scale or the units of the numbers. The header type
is part of the state type, `Data<f32, Meta>`; `Data<f32>` has no header.

A state with a header type never holds data without a header. The set that
creates the data of a `Data` or a `DataMulti` key must carry one, and so must
every take; without it the call fails (`InvalidArgument` in C). The header
type therefore needs no default, and the client never invents a header.

A change can carry a new header, which the client publishes together with the
buffer, also when the transfer is split into chunks: readers never see a new
header with an old buffer. To read the two together, use a `*_with_header`
getter: separate reads of the buffer and the header can each
see a different change. A change without a header keeps the current one.
Clearing a buffer keeps its header unless a new one is given; resetting a
`Data`, removing a `DataMulti` key or resetting the collection drops the
headers with the data. A header can also be sent alone, for data that is set.
Unlike a buffer change, the client does not acknowledge it, so a header sent
alone does not wait for the client itself, as with values. It keeps its place
after the buffer changes of its state, though: while another thread's change
to the state, or to any key of a `DataMulti`, waits for the client's
acknowledgement, the header waits behind it.

A header that the client cannot accept, because it has the wrong type, is too
large or does not decode, fails its message like invalid data does: the client
acknowledges it, reports the error and keeps its buffer and header. For an
`Add` or `Replace`, the dropped change leaves the client behind until the next
complete set or reconnect.

## Repaints and runtimes

Synchronization and repainting are separate. Server methods with an `update`
argument change state regardless of that flag; `update = true` additionally
asks the connected egui client to repaint. A delayed repaint is expressed in
seconds.

Native clients and servers each own a background thread and Tokio runtime.
WASM clients run their synchronization future on the browser executor.

## Numeric and image data

`Data` transports contiguous primitive numeric buffers (`u8` through `u64`,
`i8` through `i64`, `f32`, and `f64`) in native-endian representation, with an
optional typed header (see [Data headers](#data-headers)). This is
an efficient same-architecture transport; callers should not treat its raw byte
form as a portable file format.

A `Data` starts "not set", like a missing `DataMulti` key: its getters return
`None` on both sides (`NOT_FOUND` in C) until a set creates the data, so an
empty buffer differs from no data. Only a set creates the data; every other
change of data that is not set fails, and a reset returns the data to "not
set". A client keeps its last data across a disconnect, and the
synchronization of the next connection replaces it with the server's data, or
returns it to "not set".

Server image shapes use `[height, width]`, origins use `[y, x]`, and public
server inputs may be gray, gray-alpha, RGB, or RGBA. Client textures expose
egui's `[width, height]` size convention.

The client creates each texture in the egui context passed to
`ClientBuilder::new` when the first complete image for it arrives, so an image
sent right after connecting is never lost. Until then the client handles
report no texture, unless `Image::initialize` set a placeholder. A placeholder
is only created while no texture exists, so it never replaces a server image.

Every image dimension is limited to **65,536 pixels, inclusive**, for both `Image`
and each entry in `ImageMulti`. Rust and Python inputs, server storage and
transport packers, and client receive paths enforce the same limit before
allocating image storage or applying changes. Partial updates must also end
within that boundary and the existing image. The wire format retains `u32`
dimensions because 65,536 does not fit in `u16`.

Renderer and available-memory limits can be smaller than the library limit;
the per-side cap does not guarantee that a 65,536-square image can be
allocated or displayed.
