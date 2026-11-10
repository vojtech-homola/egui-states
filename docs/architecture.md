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
   The experimental `generate_c` writes a C header that registers the same
   states through the [C ABI](../crates/egui-states-c/README.md), which computes
   the same state ids and type hashes from names and Rust type strings at
   runtime.
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

Large numeric buffers and images are split into batches for transport and are
published to a client handle only after the complete batch is assembled.
Acknowledgements provide backpressure and release pending server work.

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

## Repaints and runtimes

Synchronization and repainting are separate. Server methods with an `update`
argument change state regardless of that flag; `update = true` additionally
asks the connected egui client to repaint. A delayed repaint is expressed in
seconds.

Native clients and servers each own a background thread and Tokio runtime.
WASM clients run their synchronization future on the browser executor.

## Numeric and image data

`Data` transports contiguous primitive numeric buffers (`u8` through `u64`,
`i8` through `i64`, `f32`, and `f64`) in native-endian representation. This is
an efficient same-architecture transport; callers should not treat its raw byte
form as a portable file format.

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
