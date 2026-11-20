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
   `Origin`. With TLS on, every connection first completes a TLS handshake,
   which sits below the WebSocket and does not change the wire protocol. The
   server tells TLS from plain connections by their first byte and answers a
   plain one itself: a browser page request with a redirect to `https://`,
   anything else with `426 Upgrade Required`.

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

## Admitting a client

The server serves one client at a time. A new connection goes through these
steps; the first that refuses it closes the connection with a WebSocket close
code from the application range and a readable reason, which the client reports
as a `CloseReason`.

1. A browser page's origin is checked during the WebSocket upgrade; a page the
   server does not allow gets HTTP 403, before any close code can be sent.
2. The handshake is checked: protocol version (4000), token (4002), then
   application version (4001). The token comes before the application version,
   and the reason sent to the client does not say what the server expects, so a
   client without the token learns nothing about the version. The server's log
   has the details.
3. If a connect-request callback is registered, the server sends
   `ConnectPending` with the time the decision may take, and the client reports
   `WaitingForApproval` and waits that long. Requests are decided one at a
   time; the next one is raised only after the previous client was served or
   refused. A refusal, a callback that fails, or a decision that is not made in
   time closes the client with 4004. The server watches a waiting client all
   the while: one that leaves ends its request, also while it waits for its
   turn, and a later answer is ignored. A waiting client sends nothing until
   the server's first synchronization message; one that does is refused.
4. Without a callback, `ClientPolicy::Reject` refuses the client with 4003 while
   another client is connected. The check is made where the run loop publishes
   connections, so no other client can connect in between.
5. The connected client, if any, is closed with 4005, and the new one is
   synchronized.

The server also closes with 4006 when it disconnects the client or fails to
synchronize a new client, and with 4007 when it stops. A client that stops
answering is dropped by the heartbeat without a close frame, as it no longer
reads; the client then reports `ConnectionLost`. A client attempt that gets no
answer at all fails with `Timeout` after the client's connect timeout.

A client built with reconnecting starts a new attempt by itself only after
causes that can pass: an unreachable server, a timeout, a lost connection or a
stopped server (4007), with a wait that doubles up to a maximum, and a busy
server (4003), with a wait of its own. A busy server frees its place once the
heartbeat drops a client that vanished. The client does not retry 4006: the
server disconnecting it would otherwise be undone at once.

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

- Single mode, the default, coalesces pending changes for a state to the
  latest value while a callback for that state is outstanding.
- Queue mode, `#[state(queue)]` on the field or `StatesCreator::queue` in a
  hand-written `State`, preserves every change and delivers them in order.

The mode is set in the state definition and passed to the server by the
generated bindings; the client type is the same in both modes.

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

Large numeric buffers, image sets, image updates and collection replacements
are split into chunks of 512 KiB and published to a client handle only after
the last chunk arrives. Small messages queued together are merged into one
frame of up to 256 messages and about 512 KiB, which saves a socket write per
message. A message larger than 64 KiB, such as a full chunk, is sent as a frame
of its own: the WebSocket library moves the unsent rest of a message after
every partial socket write, so large frames cost time quadratic in their size.
Acknowledgements provide backpressure and release pending server work.

`Data`, `DataMulti`, `Image`, `ImageMulti`, `DataTake` and `DataMultiTake`
send one acknowledged transfer at a time, and the multi states one per key: the client
acknowledges each key on its own (`AckKey`), and the keys do not wait for each
other. A change waits until the client has acknowledged the previous one of
its key, then queues its transfer and returns, so the changes one thread makes
to these states and to any other state reach the client in the order they
were made, unless the state has [low send priority](#send-priority). Changes
that different threads make to different keys can overtake each other.
With `wait`, a change returns only once the client has acknowledged it, or
the connection has ended; the call does not tell the two apart.
`is_idle` tells whether nothing is in flight, so a producer can skip a frame
on a slow link instead of waiting. A change made while no client is
synchronized is only stored, and none of these calls waits then.

When a client connects, every state is synchronized from the server's current
contents; a change made while the client was away is not replayed. A
`DataMulti`, `DataMultiTake` or `ImageMulti` is reset first and then receives
its keys one by one, so for a moment a reconnected client shows the collection
empty or with only some keys, until the last key has arrived. A partial transfer from a lost
connection is dropped on both sides.

A whole-collection replacement of a `VecState` or `MapState` larger than one
message is split at item boundaries and applied on the client only after its
last part arrives. These parts need no acknowledgement.

An image update is applied on the client once, like a set, unless `max_lines`
splits its rectangle into parts of at most that many rows from the top. Each
part is a complete update, chunked like any other, applied and acknowledged on
its own, and the client repaints after every part. The call queues the
first part and returns; each further part is sent once the client has
acknowledged the one before. A split update is therefore the one exception to
the order above: later changes of other states can reach the client before its
remaining parts, and the next change of the image waits until its last part
has been acknowledged.

Parts that are not sent yet can be dropped, and what is in flight never is. A
set drops them, because it overwrites the whole image. A forced update drops
them when its rectangle covers the rectangle of the whole split update;
otherwise it waits like any other. Either way the call still waits for the part
in flight, and the client ends with the server's pixels, because what is
dropped is overwritten.

Each key of an `ImageMulti` has its own transfer, and a set or a forced update
drops only waiting parts of its own key. A remove drops the waiting parts of
its key and is queued once nothing of that key is in flight. A reset drops the
waiting parts of every key and is queued once nothing of any key is in flight;
it first waits for the producers of every key, so none sends an image the
reset would leave on the client only. Neither needs an acknowledgement. A
disconnect invalidates pending work with a transfer generation, and reconnect
synchronization resets the client collection before sending the server's
current keys. A key's next change waits until the client has acknowledged its
synchronized image; keys the synchronization did not send are free at once.

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

A numeric take (`DataTake`, `DataMultiTake`) is acknowledged when it arrives,
so a fast producer on a slow link waits for the client instead of queuing
transfers the UI never sees; a new one replaces a buffer the UI has not taken.
With `blocking = true` the client acknowledges the transfer only when the UI
calls `take`, so the next send waits until then. A remove or reset of a
`DataMultiTake` key does not wait: the client acknowledges the blocking value
it drops. A `ValueTake` is not acknowledged unless it is blocking, which works
the same way. For numeric takes, `cache = true` also retains the last transfer
for synchronization with a newly connected client. A reconnected
`DataMultiTake` holds only the cached keys: values the client had not taken
before the disconnect are dropped.

## Send priority

A bulk state (an `Image`, `ImageMulti`, `Data`, `DataMulti`, `DataTake` or
`DataMultiTake`) can be created with low send priority: `#[state(low_priority)]`
on its field, on a substate for every bulk state inside it at any depth, or
`StatesCreator::low_priority` in a hand-written `State`. The generated server
bindings pass the priority to the state's constructor; the client behaves the
same either way, and the priority is not part of the layout hash.

The server sends the messages of these states in a lane of their own, only
while no message of a normal-priority state waits. Every message of such a
state uses the lane, so the state keeps its own order. A change of another
state made after it can reach the client first, though: after
`image.set(frame); label.set(text)` the client may show the new label before
the new frame. Where that order matters, `wait` on the bulk change keeps it.
The same holds for everything else the server sends in the normal lane, such
as a file transfer offer: it can arrive before a low-priority change made
earlier, also before the first value of a low-priority state on a new
connection. The client repaints when the low-priority changes arrive, like
after any other change, and an explicit repaint request (`server.update`) is
sent in each lane, so it also follows the low-priority changes made before it.

The lane has a window, the file transfer window of the server's profile. The
server follows every frame of the lane with a small mark, which the client
answers once it has handled everything before it, and sends while the bytes
of unanswered frames are below the window. A normal message therefore waits
behind at most one window or one 512 KiB chunk of bulk data, whichever is
larger. The window also caps the lane's throughput, at one window or one chunk
per round trip: with the default profile, 512 KiB per round trip, which is
about 5 MiB/s over a 100 ms round trip whatever the bandwidth. The
`Throughput` profile doubles that, at the cost of twice the wait for a normal
message. In the latency benchmark, a value round trip during continuous 16 MiB
`Data` sets takes 2.5 ms instead of 133 ms over a 1 Gbit/s link and 50 ms
instead of 1350 ms over a 100 Mbit/s link, while the sets move 2 % and 11 %
slower; on loopback the sets move about a quarter slower.

After eight frames of normal messages in a row, the server first sends one
waiting frame of the low-priority lane or one chunk of file data, taking turns
between the two, so a busy normal lane slows the other lanes down but never
stops them.

## File transfers

`Download<M>` and `Upload<M>` move files and buffers that are not state. They
share the WebSocket with the state messages but not its queue.

**Lanes.** Each peer's writer sends a heartbeat ping first, then any queued
state message, and a chunk of file data only when no state message waits. On
the server, the low-priority lane of [send priority](#send-priority) goes
between the two, and the starvation guard described there also lets file data
through a busy state lane. The control frames of a transfer (offer, accept, credit,
cancel, finished) travel with the state messages and keep their order, so a
state change the server sends after a finished transfer arrives after the
client sees it done.

**Credit.** The receiver grants a window of credit and returns it chunk by
chunk as its sink takes the data. The sender never has more than one window
between the two applications, wherever the bytes sit: its socket buffer, the
network or the receiver's queue. A state message therefore waits behind at
most about `window / link rate` of file data, about 2 ms at 1 Gbit/s with the
default window, and a slow disk on the receiver slows the sender instead of
filling memory. The window also caps throughput at about one window per round
trip, which matters only over long round trips.

**Profiles.** The window and the chunk size come from the server's transfer
profile, which the server announces as the first message of every connection,
so both peers use the same:

| Profile | Window | Chunk | For |
| --- | --- | --- | --- |
| `Responsive` | 64 KiB | 16 KiB | slow links, where the UI must stay responsive |
| `Balanced` (default) | 256 KiB | 32 KiB | links from 100 Mbit/s up |
| `Throughput` | 1 MiB | 128 KiB | a client on the same machine, or bulk transfers |

A smaller chunk returns credit in smaller steps, which keeps the flow steady
on slow links; a larger one costs less per chunk on fast ones.

**Offers.** Nothing flows until the receiver accepts, and the receiver
chooses the destination; a name from the sender is only a hint. The names of
the files and of the transfer are checked when an offer arrives (relative, `/`
between folders, no `.` or `..`, at most 4096 bytes), and the names of the
files, before any data flows, against the receiver's system (Windows reserved
names and characters, names that differ only in letter case). No file is
saved outside the folder the receiver chose. The folder is opened once, every
folder below it is reached by its name from the open folder above it, and a
file is created and renamed by its name in its open folder, never by a path.
A symbolic link among the folders fails the transfer: one that is there when
the first data arrives, before a file is written; one put there while the
transfer runs, at the file it is in the way of. Each
file is written to a temporary file and renamed over its target when
complete, with the sender's modification time; the rename replaces what was
there, a link included. When a transfer fails, the partial file is removed
and complete files of the set stay.

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
HTTP, or HTTPS when the server uses TLS, one download per file (folders flattened into the name) or one
uncompressed zip of all files. The zip holds each modification time twice:
exactly, in the extended timestamp that most unzip tools read, and as UTC in
the classic fields, which have no time zone and which other tools show as
local time. The ticket is the only access control, so it is
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
to the state, or to the same key of a `DataMulti`, waits for the client's
acknowledgement, the header waits behind it.

A header that the client cannot accept, because it has the wrong type, is too
large or does not decode, fails its message like invalid data does: the client
acknowledges it, reports the error and keeps its buffer and header. For an
`Add` or `Replace`, the dropped change leaves the client behind until the next
complete set or reconnect.

## Repaints and runtimes

The client repaints after every change the server makes; server setters
have no repaint argument. The server merges its messages into WebSocket
frames, and the client repaints once per frame that changed a state, after
applying all of the frame's messages, so changes made one after another
usually reach the UI in one repaint. Only the first and middle parts of a
split transfer, which the client buffers, do not repaint. A change made after
slow work, such as reading hardware between two sets, travels in a frame of
its own and repaints again; the client also repaints for its own reasons,
such as user input, so a sequence of changes is never guaranteed to appear in
one repaint. Changes that must appear together belong in one state.

`server.update` asks the client to repaint without changing a state, at once or
after a delay expressed in seconds.

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

The server stores an image in the format of its last set, or of the color of
its last fill, so a gray image takes one byte per pixel; `get` returns it in
that format, and an image that was never set has none. An update with fewer
channels is converted while it is written. An update with channels the image
lacks first widens the whole image to the smallest format that holds both:
gray gains equal color channels and a missing alpha becomes 255, so gray-alpha
and RGB meet at RGBA. Only a set or fill narrows it again. Every message
carries its own format, and the client expands each to its RGBA texture, so
the reconnect synchronization sends the stored format too and the client ends
with the same pixels either way.

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
