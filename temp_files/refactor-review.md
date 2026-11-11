# Structure review: server_core, client and transport layer

Branch `refactor-core` at `5330b03`, reviewed 2026-11-11.

**Method.** Read-only. No source file was changed and nothing was built,
tested or benchmarked, so every performance statement here comes from reading
code and still needs a measurement.

`file:line` references are relative to `crates/egui-states/src/` unless they
say otherwise. Findings are tagged **[payoff / risk]**.

---

## 1. Summary

- **One structural item is left:** Image and ImageMulti still have two
  transfer state machines that differ in lock layout, wait handling and sync
  protocol (§2, item 2).
- **One performance decision is open:** the server converts an image set to
  RGBA while it holds both the producer mutex and the pixel write lock
  (item 3).
- **Two measurements are missing:** the transfer chunk size was chosen on
  Linux loopback only, and the client's disabled Nagle algorithm has not been
  measured where it matters (item 1).
- **The rest is small:** handle layouts, twin functions, error kinds, two
  unsafe loops, stale TODOs and optional module splits.

---

## 2. Open items

Ordered by suggested priority.

1. **Measurements.** [high / none]
   - `TRANSFER_CHUNK_SIZE` = 512 KiB (`protocol/server_side.rs:15`) was
     chosen from the Linux loopback benchmark only. tungstenite moves the
     unsent rest of a message after every partial socket write, so the best
     chunk size depends on how much the socket accepts per write, which
     differs by platform. Run the `egui_states_bench` transfer benchmarks on
     Windows before the release.
   - The native client disables Nagle's algorithm. Loopback acknowledges at
     once, so it cannot show the effect; measure the latency of consecutive
     client value sets on Windows or over a real network.

2. **One transfer core for Image and ImageMulti.** [medium / high]
   - `Image` keeps pixels, transfer buffer and `SyncState` under one
     `RwLock`, so every client acknowledgement takes the pixel write lock
     (`server_core/states/image/single.rs:26-42`, `:335-336`). ImageMulti
     keeps its transfer state apart from the pixels
     (`image/multi.rs:28-37`), which is the better factoring.
   - ImageMulti clears the event again after `wait_clear` (`multi.rs:143`,
     `:209`); Image does not (`single.rs:201-216`, `:315-329`).
   - The syncs differ: Image sends one Set (`single.rs:373-400`); ImageMulti
     sends Reset plus the keys in sorted order and counts their
     acknowledgements (`multi.rs:230-240`, `:533-558`).
   - Settle these differences first, and compare the two race test sets
     (`single.rs:739`, `:795`; `multi.rs:793`, `:908`, `:1124`, `:1173`)
     before merging the cores. Do this last.

3. **The server image set converts under both locks.** [medium / medium]
   `store_rgba` runs under the producer mutex and the pixel write lock
   (`single.rs:89-95`; ImageMulti `multi.rs:365-366`), so `get_image` waits
   for the conversion. Converting into a spare buffer and swapping costs
   twice the image memory and breaks the tested allocation reuse
   (`image/common.rs:244`). Decide explicitly.

4. **Client handles with several allocations.** [low / low]
   - `VecState` and `MapState` hold `name: String` and clone it on every
     `clone()` (`client/states/vec.rs:22-28`, `:149-158`;
     `client/states/map.rs:24-30`, `:158-166`).
   - The Data, DataMulti, DataTake, DataMultiTake, Image and ImageMulti
     handles each hold a name `Arc`, a store `Arc`, a batch `Arc` and a
     sender (`client/states/data.rs:351-360`, `:664-673`;
     `client/states/data_take.rs:67-76`, `:328-337`;
     `client/states/image.rs:34-41`, `:252-259`).
   - The value handles already hold one `Arc<Shared>` each
     (`client/states/values.rs:141-147`); use the same shape here.

5. **Twin `update_value` bodies.** [low / low] Value and ValueAtomic
   (`client/states/values.rs:301`, `:392`), Static and StaticAtomic (`:470`,
   `:508`). ValueAtomic's parse error names the id (`:402`), the others name
   the path (`:308`, `:475`, `:516`). Error texts are kept by rule (§3), so
   aligning it is a separate decision.

6. **Core error kinds.** [low / low]
   - `CoreError` has two kinds, NotFound and Other
     (`server_core/error.rs:8-13`), and most core functions still return
     `String`. C therefore maps every other failure of a call to one code,
     for example OutOfRange for any `image_set` error
     (`c_abi/server.rs:785-789`). Add kinds only where a front-end needs to
     tell failures apart.
   - Redundant pre-check: `image_set_all` and `image_multi_set_all` call
     `checked_image_size` (`c_abi/server.rs:800`, `:864`), which the core's
     `checked_fill_size` already runs with the same message and code
     (`image/common.rs:151-156`).

7. **Two unsafe fill loops.** [low / low] `set_all_image` writes pixels
   through a raw pointer in both cores (`single.rs:136-146`,
   `multi.rs:410-419`), while the other pixel writers in `image/common.rs`
   are safe. One safe helper next to `store_rgba`.

8. **Stale TODO comments.** [low / none]
   - `client/client.rs:223` `// break; TODO`: report-and-continue is a rule
     (§3); state it there instead.
   - `client/states/image.rs:173`, "not sure if this is the best place to
     send ack": the ack-before-validation rule (§3) answers it.
   - `protocol/limits.rs:3`, "make these constants configurable".

9. **Finer module splits.** [low / low] Optional; only worth it if the files
   keep growing.
   - `server_core/server.rs` (848 lines) holds the registry, `ServerConfig`
     and the lifecycle.
   - `server_core/net/session.rs` could split into accept, handshake and
     batcher.
   - `client/messages.rs` could split into sender, serializer, parser and
     dispatch.
   - `BatchBuffer` lives in `client/states/data.rs`, not in its own module.

**Known trade-off, no action proposed.** Image updates go out in row strips
of up to `MSG_SIZE_THRESHOLD` = 10 MB (`single.rs:450`, `multi.rs:604`), so
the tungstenite write-buffer cost of item 1 applies to large updates. Each
strip is applied and acknowledged separately, so smaller strips would change
what the user sees.

---

## 3. Behaviour every refactor must keep

**Wire and hashes**

- The variant order of all header enums, nested tags and representative
  header bytes (`protocol/header.rs:69`, `:143`, `:282`). Data and take
  encodings are pinned by golden bytes; image packers by
  `server_core/packer_reference.rs`.
- State ids (golden test, `hashing.rs:99`).
- The layout hash and its per-kind tags. `Image` is hashed without a tag
  (`build_scripts/states_creator_build.rs:362`). The client and the generated
  bindings are compared for every kind
  (`crates/egui-states/tests/build_scripts_codegen.rs:343`), and a fixed
  value covers every kind (`:442`).

**Server ordering and acknowledgements**

- `ValueCore::set` queues the message before storing, under the write lock;
  the pending counter is +1 per send, 1 after sync, 0 after reset
  (`server_core/states/value.rs:78-123`).
- Data producers are serialized by the producer mutex, which they hold
  across the acknowledgement wait, so storage order equals send order
  (`server_core/states/data.rs:186-197`). Nothing that waits may hold the
  storage lock, because a disconnect reset takes it from the writer task.
- Until a state is synchronized, a producer only stores; the sync sends the
  stored data (`SyncState`, `data.rs:130-174`). A reset bumps the generation
  before it releases a waiting producer, and a producer of an older
  generation drops its work (`data.rs:461`, `image/single.rs:360-369`,
  `image/multi.rs:270-279`). The sync takes the producer mutex
  (`data.rs:470-475`, `single.rs:378`, `multi.rs:534`).
- The syncs of DataMulti, DataMultiTake and ImageMulti always start with a
  Reset.
- `DataMultiTake::set` queues all chunks under the state lock, so transfers
  do not interleave (`server_core/states/data_take.rs:243-266`). A take
  producer that missed a synchronization starts over.
- Image and ImageMulti: generation checks; a set supersedes a pending update
  (same key only); `force` only for the same rect (and key); controls wait
  for idle; sync sends Reset plus keys in sorted order (ImageMulti).
- A header sent alone takes the producer mutex, so it waits behind a buffer
  change in flight and cannot overtake a stored header.
- Writer shutdown order: abort the reader, `signals.reset()`,
  `on_disconnect`, then reset every acknowledgement
  (`server_core/net/session.rs:477-489`).
- Signal coalescing keeps the oldest unconsumed previous value
  (`server_core/signals.rs:84-89`). A signal reset keeps queue entries.

**Client**

- `Value::update_value` sends the acknowledgement while holding the write
  lock, before storing (`client/states/values.rs:311-313`).
- Exactly one acknowledgement per terminal message, sent before validation;
  the rule for an unknown id differs per kind (`client/messages.rs:231-362`).
- Repaint only after `Ok` and only with the update flag
  (`ServerHeader::update_flag`).
- A parse error ends the connection; a handler error is reported to the
  server and reading continues (`client/client.rs:219-226`).
- Clear, remove, reset and disconnect drop the matching partial batch
  (`client/states_creator.rs:228-243`).
- A closure getter runs under the handle's read lock, which is fair and not
  reentrant: do not call the same handle inside it.
- Textures: created on the first complete image; `initialize` only fills an
  empty slot; a full-rect update uses `set`; names and options unchanged;
  pixel bytes copied raw.

**Error texts** on all three front-ends; tests and users match on them.

**Keep separate on purpose:** error surfaces (C codes per call, Python
exception classes, Rust texts); behaviour for a missing key (Rust returns
`None`, Python raises, C returns NotFound); decode strictness; image input
rules; the three type systems; callback lifetimes.

---

## 4. Suggested plan

One PR per row, with a Conventional Commit title (AGENT.md rule 6).

| Order | PR | Items | Type | Payoff | Risk |
| --- | --- | --- | --- | --- | --- |
| 1 | Windows and non-loopback benchmark runs; adjust `TRANSFER_CHUNK_SIZE` if needed | 1 | `perf:` | high | none |
| 2 | Handle layouts, twin `update_value`, unsafe fill loops, redundant C check, TODOs | 4, 5, 6 (second part), 7, 8 | `refactor:` | low | low |
| 3 | Image set conversion outside the locks, if chosen | 3 | `perf:` | medium | medium |
| 4 | More `CoreError` kinds, where a front-end needs them | 6 | `refactor:` | low | low |
| 5 | One image transfer core | 2 | `refactor:` | medium | high |
| — | Module splits, if files keep growing | 9 | `refactor:` | low | low |

**Not recommended:**

- Making Data storage double as the message buffer.
- A shared callback worker pool for the three front-ends. Python callbacks
  need Python threads and the C API is pull-based.
- Unifying the client and server micro-batchers (their rules differ).
- Changing the chunking of image updates.
- A coalescing outbox for unacknowledged kinds. It would bound memory when a
  client is slow, but it changes which intermediate values are delivered.
