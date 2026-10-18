# Showcase

Explore values, signals, custom types, collections, numeric buffers, one-shot
transfers, and images. Native and browser GUIs share the same application;
Python and Rust servers provide the same data and actions.

## Run

Install Rust. For the Python server, also install `uv` and run `uv sync` from
the repository root. Run the commands below from `example/showcase`.

Start the Python server (building the GUI generates its bindings):

```sh
cargo build -p showcase_gui
uv run python python/run.py
```

Or start the Rust server, which generates its bindings automatically:

```sh
cargo run -p showcase_server --bin showcase-server
```

In another terminal, from the same directory, start the native GUI:

```sh
cargo run -p showcase_gui --bin showcase-gui
```

Click **Connect** and expand the feature sections to edit values, emit signals,
and explore collection and data controls. Both servers use loopback port
**8091**; run only one at a time. Stop the server with Ctrl+C.

## Browser GUI

Install Trunk, then run from the repository root:

```sh
rustup target add wasm32-unknown-unknown
trunk serve
```

Open [localhost:8090](http://localhost:8090) and click **Connect** while either
showcase server is running.

To serve the page from the showcase server on its single port instead, build it
and pass the output directory to either server, still from the repository root:

```sh
trunk build
cargo run -p showcase_server --bin showcase-server -- --web example/showcase/gui/web
# or: uv run python example/showcase/python/run.py --web example/showcase/gui/web
```

Then open [localhost:8091](http://localhost:8091) and click **Connect**.

## Read the code

- [State definitions](gui-core/src/state/mod.rs): follow each feature's module
  for its state handles and custom types.
- [UI rendering](gui-core/src/app.rs): feature controls and client interactions.
- [Python server](python/run.py) / [Rust server](rust/src/lib.rs): callbacks
  and initial data.
- [GUI launcher](gui/main.rs): native and browser entry points.
- [Python binding generation](gui/build.rs) / [Rust binding generation](rust/build.rs):
  both import the state from the shared `gui-core` application library.

After changing the state, rebuild the GUI for Python bindings or the Rust server
for Rust bindings. Generated bindings should not be edited.
