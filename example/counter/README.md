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

### C server (experimental)

The C server uses the [C ABI](../../crates/egui-states-c/README.md) and the
header that building `counter_c` generates in `c/bindings/`. Build the library
and the header, then compile and run the server. On Linux or macOS:

```sh
cargo build -p egui_states_c -p counter_c
LIB="$PWD/../../target/debug"
cc -std=c99 -I ../../crates/egui-states-c/include -I c/bindings c/main.c \
    -L "$LIB" -legui_states_c -Wl,-rpath,"$LIB" -o counter-c
./counter-c
```

With MSVC, in a developer prompt:

```bat
cargo build -p egui_states_c -p counter_c
cl /I ..\..\crates\egui-states-c\include /I c\bindings c\main.c ..\..\target\debug\egui_states_c.dll.lib /Fe:counter-c.exe
set PATH=%CD%\..\..\target\debug;%PATH%
counter-c.exe
```

## Read the code

- [State](gui/src/lib.rs): the complete shared state definition.
- [GUI](gui/src/main.rs): client setup, value edits, and reset events.
- [Python server](python/run.py) / [Rust server](rust/src/lib.rs) /
  [C server](c/main.c): initial state and callbacks.
- [Python binding generation](gui/build.rs) / [Rust binding generation](rust/build.rs) /
  [C binding generation](c/build.rs): generating the servers from the UI's state
  definition.

After changing the state, rebuild the GUI for Python bindings, the Rust server
for Rust bindings, or `counter_c` for C bindings. Generated bindings should not
be edited.
