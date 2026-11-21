# egui-states

[![crates.io](https://img.shields.io/crates/v/egui_states.svg)](https://crates.io/crates/egui_states)
[![docs.rs](https://docs.rs/egui_states/badge.svg)](https://docs.rs/egui_states)

`egui-states` synchronizes typed state between a Rust
[`egui`](https://github.com/emilk/egui) UI and a Python or Rust server over
WebSocket. Native and browser clients use the same state API. The library
handles messages and serialization so application code can work with state
and callbacks.

## Basic concepts

Define the shared state in Rust using state handles and derive `State`:

```rust
use egui_states::{Signal, Value};

#[derive(egui_states::State)]
pub struct CounterState {
    pub count: Value<i32>,
    pub reset: Signal<()>,
}
```

- `Value<T>` stores a value that either side can change. If both change it at
  the same time, the server's change wins; see
  [concurrent writes](docs/architecture.md#concurrent-value-writes).
- `Static<T>` stores a value the server controls and the client reads.
- `Signal<T>` sends a client event to server callbacks without retaining a value.
- `ValueAtomic<T>` and `StaticAtomic<T>` work like `Value` and `Static` for a
  small copyable value: a number, `bool`, or a fieldless enum with
  `#[derive(egui_states::Atomic)]`. Their `get()` is one atomic load instead
  of a lock, for values the UI reads every frame.

`Value` and `Static` fields (and their atomic variants) start from
`Default::default()`. Give a field another initial value with
`#[state(init = ...)]`; the generated server bindings start from the same value:

```rust
#[derive(egui_states::State)]
pub struct SettingsState {
    #[state(init = 0.5)]
    pub gain: Value<f32>,
    #[state(init = "ready")]
    pub status: Static<String>,
}
```

The expression is evaluated both when the bindings are generated and when the
client starts, so it must give the same value both times: it must not depend
on the environment, the clock, or the target platform.

Nest structs deriving `State` to organize larger state trees. Collections,
numeric buffers, images, custom types, and one-shot transfers are covered in the
[showcase](example/showcase/README.md). A numeric buffer or an image can carry
a typed header that describes it, `Data<f32, Meta>` or `Image<Meta>`, which the
client publishes together with the buffer or pixels; see
[Data and image headers](docs/architecture.md#data-and-image-headers).
A `Data` reads as `None` until the server sets it, so an empty buffer differs
from no data; see [numeric data](docs/architecture.md#numeric-and-image-data).

A large image or buffer delays the states the server sends after it. Mark it
`#[state(low_priority)]`, or a substate to mark every image and buffer inside
it, and the server sends it only while no other state waits; see
[send priority](docs/architecture.md#send-priority).

Generate matching server bindings from the same state definition with
`generate_python` or `generate_rust`. Construct the UI state with
`ClientBuilder::new`, which takes the app's egui context for repaints and the
server port, start the server, and call `client.connect()` to synchronize.
Rebuild bindings after changing the state definition; generated files should
not be edited.

On the client, `set` changes a value and `set_signal` also invokes server
callbacks. To bind a widget to a value, `edit` and `edit_signal` pass a copy
of it to a closure and send it only when the closure changed it:

```rust
state.count.edit_signal(|count| ui.add(egui::DragValue::new(count)));
```

The client repaints the UI after the server changes a state, once per
received batch of changes; see
[repaints](docs/architecture.md#repaints-and-runtimes).

To rebuild something from a `Data`, `VecState` or `MapState` only when the
server changed it, such as a plot of a large buffer, call `take_changed()`
once per frame before reading. It returns `true` once after each change, and
only this call clears it, so other reads don't interfere:

```rust
if state.samples.take_changed() {
    self.plot = state.samples.read(|samples| downsample(samples));
}
```

`DataMulti` has a flag per key, `take_changed(key)`, which also reports a
removed key, and `take_any_changed()` for the whole collection.

A `ValueTake<T>` delivers a one-shot value from the server, which the UI
consumes with `take()`. By default a new value replaces one the UI has not
taken yet. Mark it `#[state(queue)]` to keep every value in arrival order, up
to 100, or `#[state(queue = N)]` for another limit; when the queue is full,
the oldest value is dropped and the client reports it to the server:

```rust
#[derive(egui_states::State)]
pub struct JobState {
    #[state(queue)]
    pub log: ValueTake<String>,
}

while let Some(line) = state.log.take() {
    self.lines.push(line);
}
```

## File transfers

`Download<M>` sends files from the server to the client and `Upload<M>` from
the client to the server, alongside the synchronized state. `M` is typed
metadata sent with each transfer, `()` for none. Each transfer is an offer
that the receiver accepts or rejects, and the receiver chooses where the data
goes: into a file, into a folder for several files, or into memory.

```rust
#[derive(egui_states::State)]
pub struct FilesState {
    pub files: Download,
    pub request: Signal<String>,
}
```

A Python server answers the client's request by sending the file:

```python
def send(name: str) -> None:
    # The client only names a file; the server decides what it may send.
    server.states.files.send_file(DATA / safe_name(name))


server.states.request.connect(send)
```

and the client takes the offer in `update`, asking for a path without stopping
the UI:

```rust
if let Some(offer) = state.files.take_offer() {
    offer.accept_with_dialog();
}
if let Some(handle) = state.files.last() {
    ui.add(egui::ProgressBar::new(handle.progress().fraction().unwrap_or(0.0)));
}
```

`accept_with_dialog` and `Upload::send_file_with_dialog` come with the `rfd`
feature and show [rfd](https://docs.rs/rfd) file dialogs: on native clients a
save dialog that suggests the file's name, or a folder dialog for several
files; in a browser the download manager saves the data and uploads use the
browser's file picker. Their `_config` variants let a closure adjust the
dialog, for example `|dialog| dialog.add_filter("HDF5", &["h5"])`. On Linux
the dialogs come from the XDG desktop portal; add `rfd = "0.18"` to your own
dependencies with its `wayland` feature to attach them to the window on
Wayland (it needs the system Wayland library to build), or with `gtk3` to use
GTK instead. Without
the feature, `accept_file_with`, `accept_dir_with` and `send_file_with` take
a path from any dialog.

- **The GUI stays responsive.** File data only uses capacity the state
  messages leave free, so a transfer adds about 2 ms to a state update on a
  1 Gbit/s link. The server's `transfer_profile`, a `TransferProfile` in
  Python and in the Rust `ServerOptions`, shifts that balance: `Responsive`
  for slow links such as a VPN, `Balanced` by default, or `Throughput` for a
  client on the same machine.
- **One transfer per endpoint.** A send while an endpoint is busy fails, or
  waits with `wait=True`. Several files go as one transfer with `send_files`.
- **Files are saved atomically.** A file appears under its name only once it
  is complete, with the sender's modification time.
- **Browsers** upload `File` objects from a file picker, and download through
  the browser's download manager over HTTP (HTTPS with
  [TLS](#encrypted-connections)) from the server's port, one file at a time or
  as one zip. A browser chooses the location itself.

Transfers end when the connection does and are not resumed. See
[file transfers](docs/architecture.md#file-transfers) for the details.

## Browser clients

A browser (WASM) client is usually loaded from the machine that runs the
server. Instead of running a separate web server, create the server with the
directory of the built page (for example Trunk's output) as its `web_path`
option and call `start_web`. The page is then served on the WebSocket port, so
a single port must be reachable. `start_web` and `stop_web` switch serving on
and off at any time, before or after `start`, without affecting the connected
client; `start_web` reads the directory each time it is called, so calling it
again after a rebuild serves the new page.

A WASM client connects back to the host and port its page was loaded from (using
`wss` for HTTPS pages), so a page served this way reaches the server through any
address, tunnel, or forwarded port. Set the client builder's `host` or `addr` to
connect elsewhere. When a separate development server such as `trunk serve`
serves the page, let it forward the WebSocket to the state server; the showcase's
`Trunk.toml` shows how.

Browsers let any web page open a WebSocket to any address, including a server on
your own machine, so the server checks the page's origin (`scheme://host:port`),
which browsers state in every WebSocket request and pages cannot change. A page
may connect when the server served it, directly or through a proxy that sets
`X-Forwarded-Host` such as `trunk serve`, on an IP address or a name that only
exists on your network: one without a dot (`localhost`, `pc-a`) or ending in
`.local`, `.home.arpa`, `.internal`, or `.localhost`. Other pages are refused
before the handshake, so a site open in your browser cannot take over the
connection. To let other pages connect, list their origins in the
`allowed_origins` option, where `*` matches any scheme, part of the host, or
port: `http://myserver.corp.example:*` allows the page opened by a company
domain name, `http://localhost:3000` a page from another local server. Native clients send no
origin and are not affected; the `token` guards against other programs. An
entry of just `*` turns the check off and lets every page connect, including
local files; any site open in your browser can then connect too, so set a
`token` with it. The option is `allowed_origins` in the Rust `ServerOptions`
and the Python server's constructor.

## Network access

By default the server listens only on `127.0.0.1`, so only clients on the same
machine can connect. To accept clients from other machines, including a browser
opened elsewhere or a port published from a Docker container, pass the address
`0.0.0.0` (or a specific interface) to `start`. Native clients target
`127.0.0.1` unless the client builder's `host` (a host name or IP address) or
`addr` says otherwise.

By default connections are unencrypted, so the optional `token` and all state
data travel in clear text. On a network you do not trust, turn on
[TLS](#encrypted-connections), or keep the server on loopback and reach it
through an SSH tunnel.

### Encrypted connections

With the `tls` option, the server speaks only TLS on its port: `wss://`
WebSockets, and `https://` for the web page and browser downloads. TLS needs
no internet access, so it also works on machines that are offline.

```python
server = StatesServer(tls=True)
print(server.tls_fingerprint())  # 64 hexadecimal digits
server.start(8091, (0, 0, 0, 0))
```

Without a certificate of your own, the server generates one when it is created.
It exists only in memory, so every new server, such as each run of your
program, presents a new one; stopping and starting the same server keeps it.
`tls_fingerprint()` returns its SHA-256 fingerprint, which identifies it.

A native client connects with TLS when its builder is given a `Tls` mode (Rust
crate feature `tls`). The mode parses from text, so it can come straight from
a command-line argument:

```rust
let mode: Tls = arg.parse()?; // "any", "system", or a fingerprint
let (state, client) = ClientBuilder::<AppState>::new(ctx, 8091)
    .host("pc-a.local")
    .tls(mode)
    .build();
```

| Mode | The client accepts | Protects against |
| --- | --- | --- |
| `Tls::Fingerprint(fp)` | only the certificate with this fingerprint | listening, and someone posing as the server |
| `Tls::AcceptAny` | any certificate | listening only |
| `Tls::System` | certificates the operating system trusts for that host | listening, and someone posing as the server |

Pass the fingerprint the server reports to the client, for example as an
argument when you start it; a generated certificate, and so its fingerprint,
is new in every run of the server program. `System` suits a certificate you supply, issued by an authority
installed on the client machines. A client without TLS cannot connect to a TLS
server, nor a TLS client to a plain one, and neither falls back to an
unencrypted connection; `Client::last_error` says why.

A browser decides itself which certificates to accept. Open the page as
`https://host:port/`; a plain `http://` request is redirected there, but
whatever its address carries, such as a token, has by then travelled
unencrypted. With a
generated certificate, the browser shows a warning before the page loads, in
every run of the server program. Before accepting, compare the SHA-256 fingerprint in the
browser's certificate viewer with `tls_fingerprint()`; otherwise someone
on the network could pose as the server, and the browser would only be
protected against listening. To avoid the warning, supply a certificate that
the client machines trust and that lists the name or address the browser
uses, with `tls_cert` and `tls_key` (PEM files). Tools such as
[mkcert](https://github.com/FiloSottile/mkcert) create one, and an authority
to install on the clients, without internet access. A page served by a
separate development server such as `trunk serve` is plain HTTP, so keep TLS
off for that.

The options are `tls`, `tls_cert`, and `tls_key` in the Rust `ServerOptions`
and the Python server's constructor. The Python package always includes TLS;
Rust needs the crate's `tls` feature.

## Lost connections

The server pings the connected client every 5 s and disconnects it after 20 s
without any response, for example when the client's computer sleeps or its
network drops without closing the connection. Server operations waiting for
that client's acknowledgements are then released. Clients answer pings
automatically. Change either time, or disable the heartbeat, with
`heartbeat_interval` and `heartbeat_timeout` in the Rust `ServerOptions` or the
Python server's constructor.

By default a client connects only when `client.connect()` is called, also
after a connection ended. Build it with `.reconnect(Reconnect::default())` to
have it connect again by itself, for example while the Python server
restarts:

```rust
let (state, client) = ClientBuilder::<AppState>::new(ctx, 8091)
    .reconnect(Reconnect::default())
    .build();
client.connect();
```

After the first `connect()`, a client that cannot reach the server, loses the
connection or sees the server stop tries again after 0.5 s, waiting twice as
long after each failed attempt up to 5 s. A client refused as `Busy` tries
every 5 s. Other causes, such as a version mismatch, a refusal, or another
client taking over, wait for `connect()` as before. Between attempts
`get_state()` reports `Reconnecting`; `connect()` tries at once and
`disconnect()` stops. Each connection synchronizes every state from the
server, so what the UI changed while disconnected is replaced.

## One client at a time

The server serves one client. By default a client that connects takes over and
the connected one is closed. With `client_policy=ClientPolicy.Reject` in the
Python server's constructor (`client_policy` in the Rust `ServerOptions`) the
connected client stays and newcomers are refused as busy. A client that
vanished without closing its connection, such as a sleeping laptop, then keeps
its place until the heartbeat drops it, 20 s by default; with the heartbeat
disabled, call `disconnect_client` to free the place.

To decide per client, register a connect-request callback. The server then asks
it for every client that passed the handshake, instead of applying the policy:

```python
from egui_states import ConnectRequest


def decide(request: ConnectRequest) -> bool:
    return not measurement_running  # True lets request.address in


server.on_connect_request(decide)
```

The callback may block, for example while the user of the connected client
answers a prompt, for up to `connect_request_timeout` (5 s by default); a
client not decided by then is refused. Check `request.is_pending()` to stop
waiting when the client leaves or the time runs out. Requests are decided one
at a time, and the connected client is unaffected meanwhile. A waiting client
sends nothing, and what its UI changes meanwhile is dropped, as while
disconnected. In Rust the callback is `StateServer::on_connect_request`.

On the client, `get_state()` reports `Connecting` during an attempt,
`WaitingForApproval` while the server's callback decides, then `Connected`.
After `Failed`, `Disconnected` or `Reconnecting`, `last_reason()` returns why
as a `CloseReason`, such as `Unreachable`, `Timeout`, `VersionMismatch`,
`TokenRejected`, `Busy`, `Refused`, `Replaced`, `ClosedByServer`,
`ServerStopped`, or `ConnectionLost`, and `last_error()` describes it in words.
An attempt the server does not answer fails after the client builder's
`connect_timeout` (10 s by default). The server sends its reasons in the
WebSocket close frame, so browser clients receive them too.

## Examples

Start with the counter, then explore individual features in the showcase.
Each example includes run commands and links to its state, UI, and server code.

| Example | Demonstrates | GUI |
| --- | --- | --- |
| [Counter](example/counter/README.md) | A shared count and reset signal | Native |
| [Showcase](example/showcase/README.md) | State types, collections, buffers, and images | Native and browser |

Both examples work with either a Python or Rust server and generate bindings
when built.

## Cargo features

- `client` (default): egui state handles and `ClientBuilder`.
- `server`: native Rust server API.
- `build_scripts`: Python and Rust binding generators.
- `python`: PyO3 support for building the Python extension.
- `tls`: encrypted connections for the server and native clients (see
  [encrypted connections](#encrypted-connections)).

See the [API reference](https://docs.rs/egui_states),
[synchronization model](docs/architecture.md), and [test commands](tests/README.md)
for further details.
