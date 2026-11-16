# Counter

A minimal native GUI with a shared `count: Value<i32>` and a `reset: Signal<()>`.
Edit the count to synchronize it and log the change on the server; reset asks
the server to restore zero. Choose either a Python or Rust server.

## Run

Install Rust. For the Python server, also install `uv` and run `uv sync` from
the repository root. Run the commands below from `example/counter`.

Start the Python server (building the GUI generates its bindings):

```sh
cargo build -p counter_gui
uv run python python/run.py
```

Or start the Rust server, which generates its bindings automatically:

```sh
cargo run -p counter_server --bin counter-server
```

In another terminal, from the same directory, start the GUI:

```sh
cargo run -p counter_gui --bin counter-gui
```

Click **Connect**, edit the count, and try **Reset**. Every server uses loopback
port **8092**; run only one at a time. Stop the server with Ctrl+C.

## Read the code

- [State](gui/src/lib.rs): the complete shared state definition.
- [GUI](gui/src/main.rs): client setup, value edits, and reset events.
- [Python server](python/run.py) / [Rust server](rust/src/lib.rs): initial state
  and callbacks.
- [Python binding generation](gui/build.rs) / [Rust binding generation](rust/build.rs):
  generating the servers from the UI's state definition.

After changing the state, rebuild the GUI for Python bindings or the Rust server
for Rust bindings. Generated bindings should not be edited.
