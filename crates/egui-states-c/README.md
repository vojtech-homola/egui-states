# egui-states C ABI

`egui_states_c` exposes the egui-states server through a C ABI. It is an
experimental, low-level gateway for servers and server wrappers written in
other languages, such as C++ or C#. The [Rust](../../README.md) and Python
servers remain the complete, idiomatic implementations, and nothing here
changes them: the support code in `egui_states` is behind the opt-in `c_abi`
feature, and this crate is not a default workspace member.

The ABI registers states at runtime from Rust type strings, moves values
through type-checked writers and readers, and delivers client changes by
polling. Everything is declared and documented in
[`include/egui_states.h`](include/egui_states.h), which cbindgen generates
from this crate.

## Build

```sh
cargo build -p egui_states_c --release
```

The libraries are written to `target/release`:

| Platform | Shared library | Static library |
| --- | --- | --- |
| Linux | `libegui_states_c.so` | `libegui_states_c.a` |
| macOS | `libegui_states_c.dylib` | `libegui_states_c.a` |
| Windows (MSVC) | `egui_states_c.dll` with import library `egui_states_c.dll.lib` | `egui_states_c.lib` |

A static library also needs the system libraries that Rust's standard
library uses; `cargo rustc -p egui_states_c --release -- --print native-static-libs`
lists them.

On Linux and Windows, this workspace builds x86-64 code for `x86-64-v3` CPUs
(see `.cargo/config.toml`). Remove that setting to build a library for older CPUs.

## Use

States are named by their full dotted path and typed with the Rust spelling
the client uses. Struct and enum definitions are registered once, before the
states that use them:

```c
#include "egui_states.h"

egui_states_server_t *server;
egui_states_writer_t *writer;
egui_states_reader_t *reader;
uint64_t count_id, reset_id;

egui_states_server_new(NULL, &server);  /* NULL: no application version check */
egui_states_writer_new(&writer);
egui_states_reader_new(&reader);

egui_states_write_i32(writer, 0);       /* initial value */
egui_states_server_add_value(server, "root.count", "i32", writer, 0, &count_id);
egui_states_server_add_signal(server, "root.reset", "()", 0, &reset_id);
egui_states_server_finalize(server);

egui_states_server_subscribe(server, count_id, 0);
egui_states_server_subscribe(server, reset_id, 0);
egui_states_server_start(server, 8092, "127.0.0.1", NULL);

for (;;) {
    egui_states_event_t *event;
    uint64_t id;
    if (egui_states_server_wait_event(server, 500, &event) != EGUI_STATES_OK) {
        continue;  /* EGUI_STATES_ERROR_TIMEOUT */
    }
    egui_states_event_id(event, &id);
    if (id == reset_id) {
        egui_states_writer_clear(writer);
        egui_states_write_i32(writer, 0);
        egui_states_value_set(server, count_id, writer, EGUI_STATES_UPDATE);
    }
    egui_states_event_free(event);  /* releases the state for its next event */
}
```

Every fallible function returns an `egui_states_status_t`;
`egui_states_last_error()` describes the last failure on the calling thread.
Checks are omitted above for brevity.

Writing names, types, and initial values by hand is error-prone. A build
script can generate them from the Rust state definition with
`egui_states::build_scripts::generate_c` (features `build_scripts` and
`c_abi`), which writes a header with the state
ids, type strings, layout hash, and a `<prefix>_register(server)` function.
The [C counter example](../../example/counter/README.md#c-server) uses it.

## Values

Writers record values in declaration order, and every function that takes a
writer checks those writes against the target type. Readers check each read
the same way:

- Structs, tuples, fixed-size arrays, and `()` have no header of their own;
  write or read their fields in order.
- `Option<T>` is `egui_states_write_none`, or `egui_states_write_some`
  followed by the value.
- `Vec<T>` and `HashMap<K, V>` are `egui_states_write_len` followed by the
  elements or key/value pairs.
- Enums are written as their variant index in declaration order, not as their
  discriminant.

## Events and threads

`egui_states_server_wait_event` returns the next change of a subscribed
`Value`, `Signal`, or system event (`EGUI_STATES_ID_*`). An event holds a claim
on its state until it is freed, so the changes of one state are handled one at
a time even when several threads wait. `egui_states_server_close_events` wakes
every waiter before shutdown.

Server functions are thread-safe, except `egui_states_server_free`. While a
client is connected, `Data`, `DataMulti`, `Image`, `ImageMulti`, and take
operations may block until the state's previous transfer is acknowledged or
the client disconnects.

## Development

The header is generated; after changing an exported function, regenerate it
and commit the result:

```sh
EGUI_STATES_BLESS=1 cargo test -p egui_states_c --test header
```

`cargo test -p egui_states_c` checks the header, the argument handling, and a
round trip of every state kind with a Rust client. The C counter example is
compiled and run by `cargo test -p counter_c`.
