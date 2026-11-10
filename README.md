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

Nest structs deriving `State` to organize larger state trees. Collections,
numeric buffers, images, custom types, and one-shot transfers are covered in the
[showcase](example/showcase/README.md). A numeric buffer can carry a typed
header that describes it, `Data<f32, Meta>`, which the client publishes
together with the buffer; see [Data headers](docs/architecture.md#data-headers).

Generate matching server bindings from the same state definition with
`generate_python` or `generate_rust`. Construct the UI state with
`ClientBuilder::new`, which takes the app's egui context for repaints and the
server port, start the server, and call `client.connect()` to synchronize.
Rebuild bindings after changing the state definition; generated files should
not be edited.

For other languages, the experimental [C ABI](crates/egui-states-c/README.md)
exposes a low-level server for wrappers to build on, and `generate_c` writes a
C header that registers the same state tree. It is opt-in and does not change
the Rust or Python servers.

On the client, `set_signal` changes a value and invokes server callbacks.
Server setters can request an egui repaint with their `update` flag.

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
and the Python server's constructor, and `egui_states_server_set_allowed_origins`
in C.

## Network access

By default the server listens only on `127.0.0.1`, so only clients on the same
machine can connect. To accept clients from other machines, including a browser
opened elsewhere or a port published from a Docker container, pass the address
`0.0.0.0` (or a specific interface) to `start`. Connections are unencrypted
WebSockets, so the optional `token` and all state data travel in clear text;
on an untrusted network, keep the server on loopback and reach it through an
SSH tunnel. Native clients target `127.0.0.1` unless the client builder's `host`
(a host name or IP address) or `addr` says otherwise.

## Lost connections

The server pings the connected client every 5 s and disconnects it after 20 s
without any response, for example when the client's computer sleeps or its
network drops without closing the connection. Server operations waiting for
that client's acknowledgements are then released. Clients answer pings
automatically. Change either time, or disable the heartbeat, with
`heartbeat_interval` and `heartbeat_timeout` in the Rust `ServerOptions` or the
Python server's constructor, or with `egui_states_server_set_heartbeat` in C.

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
- `c_abi` (experimental): support for the `egui_states_c` C ABI library; with
  `build_scripts`, also the `generate_c` header generator.

See the [API reference](https://docs.rs/egui_states),
[synchronization model](docs/architecture.md), and [test commands](tests/README.md)
for further details.
