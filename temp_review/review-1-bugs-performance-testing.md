# egui-states review 1: bugs, performance, testing

Reviewed at commit `e3643b5` (main, 2026-10-18), workspace version 0.16.0,
`PROTOCOL_VERSION = 7`.

## Scope and method

I read every non-generated source file in the workspace (about 46k lines:
`crates/egui-states`, `egui-states-macros`, `egui-states-c`, the Python
package, the test crates, and both examples), then ran:

- `cargo test --locked --no-fail-fast -- --test-threads=1` (default members):
  247 tests, all green, 58 s wall clock.
- `cargo clippy -p egui_states --features "server build_scripts" --all-targets`:
  84 warnings in the library (no errors).
- `cargo fmt --all --check`: clean.
- A scratch crate outside the repository with four targeted tests to confirm
  suspected defects.

B1 to B7, B9, and several B15 items have since been fixed on the
`fix-easy-bugs` branch, and B8, B10, and B11 on the `review-bugs-from-review`
branch (PR #74), all but B7 with a regression test. They are no longer listed
here. The remaining findings keep their original numbers, so the references
in review 2 still match. Testing gaps closed since the review (CI, the clippy
baseline) are removed as well; the numbers above describe the reviewed commit.

Each defect entry names the file, the mechanism, the impact, and a suggested
fix. A prioritized action list closes the document.

---

## Part A. Defects found by inspection (high confidence, not executed)

### B15. Minor API and robustness issues

- The extension declares `gil_used = false` (free-threaded Python), and
  `image_set`/`data_set` read NumPy memory with the GIL released. A concurrent
  Python writer to the same array produces torn reads. Document it, or hold a
  read lock on the buffer (`PyBuffer` only guarantees the memory stays valid).
- Handshake token comparison is not constant-time and the token travels in
  clear text (no TLS anywhere). Acceptable for loopback, worth stating in the
  README's security notes.
- There is no bound on concurrently pending handshakes (`JoinSet` in
  `core.rs`), only a 10 s timeout each; a loopback-only tool can live with
  that, but it should be a documented limit.
- `Vec.remove_item` and `Dict.remove_item` in Python discard the removed item
  that `list_del_item` and `map_del_item` return. Returning it changes the
  public Python API.

---

## Part B. Performance review

The architecture is sound for its stated use (one egui client, one server,
same machine or LAN): postcard framing, micro-batching of small messages, 10 MB
chunks with acknowledgements for large transfers, and `Bytes` to avoid copies
on the receive path. The points below are where real work is wasted.

### P1. Numeric data is copied twice on the server

`Data::set` copies the caller's buffer into the stored `Vec`, then `pack_data`
copies the stored bytes again into one or more `FastVec` messages, which
`to_bytes` wraps without a further copy. For a 100 MB `set` that is 200 MB of
memcpy plus the allocation churn. Storing the state as `Bytes` (or an
`Arc<Vec<u8>>`) and building messages as header `Bytes` + payload `Bytes::slice`
would make the payload zero-copy; tungstenite accepts `Bytes` directly. The
same applies to `DataMulti`, `DataTake` (`slice.to_vec()` for the cache), and
to image packing, where `append_data` copies the caller's pixels into the
message after they were already converted into the RGBA store.

### P2. Images are stored expanded to RGBA on the server

A grayscale camera frame costs 4x its size in server memory and the full RGBA
copy is what reconnect synchronization sends. Storing the original format plus
a tag and expanding only on the client would cut server memory and reconnect
traffic for 1, 2, and 3 channel images.

### P3. Synchronization runs on the Tokio worker thread

`core::run` calls every `sync()` inline after the handshake. For `ImageMulti`
that packs every stored image; for `Data` it copies the whole buffer. During
that time one of the two runtime workers is blocked, so accepts and reads
stall. `tokio::task::spawn_blocking` (or doing the packing on the producer side
once and caching the frames) keeps the runtime responsive.

### P4. Signal scheduling is linear in the number of pending changes

Single-mode ids hold one scheduling slot however often they change, but
queue-mode ids hold one entry per queued change, and `get` scans linearly past
entries of blocked ids, so a queued value with a slow callback makes every
`get` cost grow with its backlog. Parking blocked ids and
keeping ready ids in an ordered set keyed by a per-change sequence number
makes `get` O(log k) in the number of distinct ready ids while keeping the
cross-id arrival order.

### P5. Per-frame clones on the client

`Value::get()` clones `T`; the showcase calls `title.get()` (a `String`) and
`summary.get()` every frame. `read()` exists and should be the documented
default for non-`Copy` types; `Diff` already covers the edit case. Consider
`get_ref()`-style guards or `Arc<T>` snapshots for large values.

### P6. Threads per process

Each server owns an OS thread plus a two-worker runtime (keep-alive one hour),
each client another three threads, plus the signal workers. A server embedded
in a Python process alongside several clients in tests accumulates dozens of
idle threads. A `current_thread` runtime for the client and a configurable
worker count for the server would help.

### P7. Batching copies

`DataReceiver::next` concatenates up to ten queued messages (up to 10 MB) by
`extend_from_data`, i.e. memcpy, to emit one frame. For bursts of 1 MB values
that is a 10 MB copy per frame. Since tungstenite needs a single `Bytes` per
message anyway, the alternative is to lower the batching ceiling for large
payloads (batch only messages below, say, 64 KB) so that large values go out
as their own frame without a copy.

### P8. Static web page serving

Each asset is served on a fresh TCP connection (`Connection: close`), the head
is peeked with a 5 ms polling loop, and files are sent uncompressed. For a
multi-megabyte WASM bundle on localhost this is fine; on a LAN it is slow
enough that users will notice. (Serving the files as they are is a deliberate
decision recorded in the project memory; keep-alive would still be a cheap
win.)

### P9. Compile-time and dependency footprint

The client pulls `tokio` with `rt-multi-thread`, `futures-util`, and `sha2`.
`sha2` is used only to derive ids and layout hashes at
startup; a smaller stable hash (for example `siphasher` with fixed keys, or a
vendored FNV-1a 64) would remove a dependency without changing the stability
guarantee, at the cost of a protocol bump (ids are on the wire). Worth doing
only together with another protocol change.

---

## Part C. Testing review

### What is good

- 275 Rust tests and 54 Python tests, all passing, in under a minute for the
  default members. Unit tests sit next to the code and are written against
  behavior ("a claimed id is not handed out again until it is passed back")
  rather than implementation details.
- The 19 shared client/server scenarios run the same native client against
  both the Rust and the Python server from one registry; wire-shape scenarios
  cover every value shape both ways.
- Codegen is tested for determinism, timestamp preservation, map ordering,
  keyword escaping, non-finite floats, conflicting definitions, and
  compile-fail diagnostics of the macros.
- The C ABI has boundary tests for null pointers, flags, UTF-8, short
  buffers, panics, and a golden header check; the C example is compiled with
  `-Wall -Wextra -Werror -pedantic` and as C++.
- Golden tests pin state ids, type hashes, header variant indexes, and the
  layout hash of a fixed schema, which is exactly what protects a hashed wire
  protocol from accidental breakage.
- Tests that can block run in a child process with a deadline and captured
  output; there are no expected-failure markers.

### Gaps left around the fixed bugs

1. `Acknowledge::reset` has tests for `DataMulti`, `ValueCore`, `Image`, and
   `ImageMulti`, but there is no table-driven test over every state kind.
2. No concurrent-producer test for `DataMultiTake::set`. The fix holds the
   lock across the send loop; a deterministic test needs a hook in the sender,
   and a timing-based one with 10 MB chunks would rarely catch a regression.

### Structural gaps

- The WASM client (`client/websocket_wasm.rs`, `spawn_local` path in
  `client.rs`) is only compile-checked in CI; none of it runs in a test.
  `wasm-pack test --headless --chrome` against a native server would cover
  it.
- CI runs a pinned `ruff` through `uvx`, but `ruff` is still not in the `dev`
  dependency group, so `uv run ruff` fails in a fresh environment. The Python
  code carries `# ty: ignore[...]` comments, so a type checker is in use
  somewhere but is declared neither in the group nor in CI.
- `test_support::isolated()` re-executes the test binary with `--exact` for
  each isolated test, which doubles its runtime. `cargo nextest` gives
  process-per-test isolation, per-test timeouts, retries, and JUnit output
  natively and would let you delete the helper.
- Several tests rely on wall-clock sleeps (`thread::sleep(300 ms)` in
  `callbacks_for_one_state_do_not_overlap_across_a_reconnect`, 150 ms
  negative-assertion windows in the blocking scenarios, 50 ms before sending
  signals). They pass reliably on this machine; on a loaded CI runner they are
  the first candidates for flakiness. Where possible replace fixed waits with
  observable conditions (the fixture already has `phase`).
- The large transfer tests allocate hundreds of MiB and force
  `--test-threads=1` for the whole workspace. Making `MSG_SIZE_THRESHOLD`
  and `VALUE_MAX_SIZE` overridable under `cfg(test)` (or exposing them as
  server options, which review 2 recommends anyway) would let those tests
  use small thresholds and run in parallel.
- No fuzzing. `MessagesParser`, `SocketReader`, `c_abi::type_parser`,
  `c_abi::Reader`/`tree::walk`, and `pyparsing::deserialize_py` are
  textbook fuzz targets with existing hand-written malformed-input tests that
  can seed a corpus. `cargo fuzz` with three targets would have found the
  client length-handling panics fixed on `fix-easy-bugs`.
- No Miri run over the `unsafe` helpers (`image_core_common.rs` pointer
  loops, the `copy_nonoverlapping` + `set_len` patterns in `client/data.rs`,
  `data_take.rs`, `server/data.rs::data_from_bytes`, and the atomic locks).
  They look correct, but they are exactly the code Miri is for.
- No property tests. `FastVec` has a hand-rolled xorshift model test; signal
  coalescing, `split_collection`, and the `InitValue` canonical ordering are
  natural `proptest` candidates.
- No soak or leak tests: connect/disconnect a few hundred times and check
  thread counts, memory, and that the signal scheduling list returns to size zero.
- No benchmarks. A small `criterion` suite (pack_data per size, image packing
  per format, `serialize_py` per type shape, `DataReceiver` batching, signal
  `get` under load) would make P1 to P4 measurable and would catch
  regressions in the hot paths.
- The `.pyi` stub is maintained by hand and had drifted (two return types,
  since fixed). A test that introspects `StateServerCore` and compares the
  method set and arities with the stub prevents that.
- Cross-version tests: the handshake rejection path is tested, but there is no
  recorded-frame golden file per `ServerHeader` kind. The current golden
  tests pin discriminants; a byte-level fixture per message kind would pin
  field order and encodings too.

---

## Part D. Code quality and maintenance observations

- Five near-identical transfer state machines live side by side: `Data`,
  `DataMulti`, `DataTake`, `DataMultiTake` (server about 1.4k lines, client
  about 1.9k) and `Image`/`ImageMulti` (server about 2.5k, client about 1k).
  Each re-implements chunking, acknowledgement counting, sync generations,
  and buffering with small differences. Two of the bugs fixed on
  `fix-easy-bugs` (`DataMulti` had no `reset`, `DataMultiTake::set` released
  its lock before sending) were "one sibling forgot what the others do" bugs.
  A generic ordered-transfer core parameterized over a payload codec would
  remove roughly half of that code.
- `server_core/server.rs` repeats the same 20-line registration body twelve
  times; `python/pyserver.rs` repeats the buffer extraction eight times. Both
  are macro or helper material.
- Client diagnostics use `println!` in debug builds and are silent in release
  (`client/client.rs::print_error`); the WASM path already uses `log`. Using
  `log` (or `tracing`) everywhere lets applications route messages.
- `pub mod c_abi` is `#[doc(hidden)]` but public, and `egui_states_c` depends
  on it. That is a public API in disguise and a semver hazard for the
  `egui_states` crate; a separate internal crate would make the boundary
  honest.
- The `Queue`/`NoQueue` type parameter on client types encodes a server
  callback policy in the client's type system (see review 2).
- Recent history shows very large PRs (#64 to #66 add roughly 20k lines
  together). The code is consistent and well commented, but the duplication
  above suggests that generalizing before the next transfer kind is added
  would pay off.

---

## Part E. Prioritized action list

1. Zero-copy payloads for `Data`/`Image` (P1), move sync packing off the
   runtime thread (P3), O(1) signal scheduling (P4).
2. Fuzz targets for the parsers, Miri over the unsafe helpers, and a criterion
   suite; add `ruff` and the type checker to the dev group.
3. Refactor the transfer state machines into one generic implementation
   before the next state kind is added.
