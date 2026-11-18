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

## IPython console

After building the Python bindings, start IPython from the repository root:

```sh
uv run ipython
```

Run the same script with `--interactive` to return to the prompt with
`server` and `states` available:

```ipython
%run example/showcase/python/run.py --interactive
states.values.count.get()
states.values.count.set(42, update=True)
```

Connect the GUI as usual. Interactive mode also accepts `--port` and the `--web`
switch to serve the built browser GUI.
It leaves the server running and preserves the console's signal handlers, so
Ctrl+C interrupts console input. Stop the server explicitly when finished or
before rerunning:

```python
server.stop()
```

To load the setup functions without starting a server, use
`%run -n example/showcase/python/run.py`, then call `server = setup_server()` and
`server.start(8091)` yourself.

## Browser GUI

Install Trunk, then run from the repository root:

```sh
rustup target add wasm32-unknown-unknown
trunk serve
```

Open [localhost:8090](http://localhost:8090) and click **Connect** while either
showcase server is running. The page connects to its own origin, and Trunk
forwards that WebSocket to the server on 8091 (see the repository's
`Trunk.toml`).

To serve the page from the showcase server on its single port instead, build it
and enable serving, still from the repository root:

```sh
trunk build
uv run python example/showcase/python/run.py --web
# or:
cargo run -p showcase_server --bin showcase-server -- --web example/showcase/gui/web
```

The Python server always uses `example/showcase/gui/web`, resolved relative to
`run.py`, so its web directory is independent of the current working directory.
`--web` enables serving and takes no path argument.

Then open [localhost:8091](http://localhost:8091) and click **Connect**.

## Encrypted connection

Both servers take `--tls` to encrypt connections with a certificate generated
for this run, and print its fingerprint. Pass the fingerprint to the native
GUI, which then accepts only that certificate (`--tls any` accepts any):

```sh
uv run python example/showcase/python/run.py --tls
# prints: TLS certificate fingerprint (SHA-256): 4f2a...
cargo run -p showcase_gui --bin showcase-gui -- --tls 4f2a...
```

The GUI also takes `--host` and `--port`. With `--web --tls`, open
[https://localhost:8091](https://localhost:8091); the browser warns about the
generated certificate, whose SHA-256 fingerprint in the certificate viewer
should match the printed one. Trunk serves plain HTTP, so keep TLS off with
`trunk serve`.

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
