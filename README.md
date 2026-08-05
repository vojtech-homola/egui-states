# egui-states

`egui-states` synchronizes typed application state between an
[`egui`](https://github.com/emilk/egui) UI and a server. The UI is the Rust
client; the server can be written in Python or Rust. Both native and WebAssembly
egui clients use the same state API and communicate with the server over a
WebSocket connection.

The project is useful when an egui application is primarily a view and control
surface for work performed elsewhere—for example, a Python data-processing
application, an instrument controller, or a separate Rust service. Application
code reads and writes typed state handles instead of defining its own messages,
serialization, and synchronization logic.

## How it works

1. Define the UI's state tree as Rust structs whose fields are `egui-states`
   handles.
2. Derive `State` so the client can construct the tree and calculate its stable
   layout hash.
3. Generate matching Python or Rust server bindings from that same Rust type in
   a build script.
4. Start the server and connect the egui client. Updates are serialized,
   type-checked, and applied to the matching state on the other side.

The main state types describe both the stored data and its direction:

- `Value<T>` is stored on both peers and can be changed in either direction.
- `Static<T>` is controlled by the server and read by the client.
- `Signal<T>` is a transient client-to-server event. Signals can coalesce
  pending values or queue every value.
- `ValueTake<T>` and `DataTake<T>` are one-shot server-to-client transfers.
- `VecState<T>` and `MapState<K, V>` synchronize collections.
- `Data<T>` and `DataMulti<T>` efficiently synchronize numeric buffers; Python
  exposes them as NumPy arrays.
- `Image` synchronizes complete images or rectangular updates to an egui
  texture.

State paths follow the Rust field hierarchy. A field named `counter` on the
root state is registered as `root.counter`; a field inside `controls` becomes
`root.controls.<field>`.

## Minimal Python-server workflow

Server generation must import the same Rust state type as the UI. In a real
workspace, put that type in a small shared library crate and use it as both a UI
dependency and a build dependency. The complete arrangement is demonstrated in
[`example/`](example/); the essential pieces are shown below.

Define the state shared by the UI and generator:

```rust
use egui_states::Value;

#[derive(egui_states::State)]
pub struct AppState {
    pub counter: Value<i32>,
}
```

Generate a Python package from it in the UI application's `build.rs`:

```rust
use egui_states::build_scripts::generate_python;
use ui_state::AppState;

fn main() {
    generate_python::<AppState>("python/states_server").unwrap();
}
```

The build script needs `egui_states` with the `build_scripts` feature and the
shared state crate as build dependencies:

```toml
[build-dependencies]
egui_states = { version = "0.15", features = ["build_scripts"] }
ui_state = { path = "../ui-state" }
```

Construct and connect the client from the egui application:

```rust
use egui_states::ClientBuilder;
use ui_state::AppState;

let (states, client) = ClientBuilder::<AppState>::new()
    .context(egui_context.clone())
    .build(8091);
client.connect();

// Inside the UI:
if ui.button("Increment").clicked() {
    states.counter.set_signal(states.counter.get() + 1);
}
```

After building the UI crate, add the generated package to Python's import path
and run the server:

```python
from states_server import StatesServer

server = StatesServer(port=8091)
server.states.counter.connect(lambda value: print("counter:", value))
server.start()

server.states.counter.set(41, update=True)
input("Server is running; press Enter to stop.\n")
server.stop()
```

`set_signal` on the client updates the value and invokes connected server
callbacks. The server's `update=True` asks egui to repaint after applying its
new value.

To generate a native Rust server instead, use
`egui_states::build_scripts::generate_rust` and enable the `server` feature in
the server crate. See [`example/rust`](example/rust) for a complete server.

## Custom types

Use `#[egui_states::typed]` to make a struct or fieldless enum available to
`egui-states`. The attribute implements `Typed` and derives Serde serialization
through the library's Serde re-export, so the consuming crate does not need a
direct `serde` dependency.

```rust
#[egui_states::typed]
#[derive(Clone, Debug, PartialEq)]
struct Settings {
    enabled: bool,
    label: String,
}
```

Add `egui_states::InitialValue` when the type is used by a `Value` or `Static`
field whose default must be emitted into generated server bindings:

```rust
#[egui_states::typed]
#[derive(Clone, Debug, Default, PartialEq, egui_states::InitialValue)]
struct Settings {
    enabled: bool,
    label: String,
}
```

The `typed` attribute replaces the former
`#[derive(serde::Serialize, serde::Deserialize, egui_states::Typed)]` syntax.
