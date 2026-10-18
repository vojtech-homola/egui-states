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

- `Value<T>` stores a value that either side can change.
- `Static<T>` stores a value the server controls and the client reads.
- `Signal<T>` sends a client event to server callbacks without retaining a value.

Nest structs deriving `State` to organize larger state trees. Collections,
numeric buffers, images, custom types, and one-shot transfers are covered in the
[showcase](example/showcase/README.md).

Generate matching server bindings from the same state definition with
`generate_python` or `generate_rust`. Construct the UI state with
`ClientBuilder`, start the server, and call `client.connect()` to synchronize.
Rebuild bindings after changing the state definition; generated files should
not be edited.

For other languages, the experimental [C ABI](crates/egui-states-c/README.md)
exposes a low-level server for wrappers to build on, and `generate_c` writes a
C header that registers the same state tree. It is opt-in and does not change
the Rust or Python servers.

On the client, `set_signal` changes a value and invokes server callbacks.
Server setters can request an egui repaint with their `update` flag.

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
