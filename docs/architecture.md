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
   checks are configured.

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
  update also schedules registered server callbacks.
- `Static`, collections, `Data`, and `Image` are controlled by the server and
  read by the client.
- `Signal` carries an event from the client to server callbacks without
  retaining a value. A Rust server may also emit it directly to its local
  callbacks.
- `ValueTake` and `DataTake` carry a server-to-client value that the client
  consumes once. `DataMultiTake` maintains the same lifecycle independently
  for each numeric key.

## Callback delivery modes

Signals and signaled `Value` changes use one of two server-side modes:

- Single mode (`NoQueue`, the default) coalesces pending changes for a state to
  the latest value while a callback for that state is outstanding.
- Queue mode (`Queue`) preserves every change and delivers them in order.

Callbacks for different states may run concurrently when the server has more
than one signal worker. A callback handle owns its registration; dropping the
handle disconnects it.

`connect_previous` receives the value last delivered to that callback. In
single mode this is not necessarily the immediately preceding network update:
if `a -> b -> c` coalesces to `c`, the callback receives `(c, a)`.

## Transfers and acknowledgements

Large numeric buffers and images are split into batches for transport and are
published to a client handle only after the complete batch is assembled.
Acknowledgements provide backpressure and release pending server work.

All keys in an `ImageMulti` share one ordered transfer stream. Sets and updates
that require multiple messages finish before a later remove or reset is sent;
controls do not cancel preceding chunks. A disconnect invalidates pending work
with a transfer generation, and reconnect synchronization resets the client
collection before sending the server's current keys. Each synchronized image
must be acknowledged before subsequently queued work is released.

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
egui's `[width, height]` size convention after initialization.

Every image dimension is limited to **65,536 pixels, inclusive**, for both `Image`
and each entry in `ImageMulti`. Rust and Python inputs, server storage and
transport packers, and client receive paths enforce the same limit before
allocating image storage or applying changes. Partial updates must also end
within that boundary and the existing image. The wire format retains `u32`
dimensions because 65,536 does not fit in `u16`.

Client `Image::initialize` returns `Result<(), String>` and rejects an oversized
initial image before creating a texture. Renderer and available-memory limits
can be smaller than the library limit; the per-side cap does not guarantee that
a 65,536-square image can be allocated or displayed.
