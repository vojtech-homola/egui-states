# Review of the synchronized states

Date: 2026-11-19, at commit `9d2a1fb` (branch `stetes-review`). Items marked
**Done** were implemented afterwards on the same branch (`b53a0f1`,
`294148b`, `93caba3`, `a317fb9`, `dbe4f7f`, `e9e17aa`, `6d87258`,
`53b228b`, `4f13a2d`, `6ec4de2`, `e80c526`, `73cbabf`, `00f5834`,
`5984993`, `a2a8f39`, `f7d4280`, `144b0ee`, `2c8a4aa`, `b3c8690`,
`a34ee4e`, `8811c04`, `ab7b61c`, `0fde312`).

**Verdict:** the set of states is close to complete for a Python-backend UI,
and the transport side (priority lanes, headers, transfers) is in better shape
than the day-to-day API. The usability gaps are mostly in the small, frequently
used calls, not in missing state kinds.

This review comes from reading the code only. Nothing was built or run, so the
behaviour described for Python `Vec` and `Map` iteration is what the code
implies, not what a test showed.

## Biggest usability gaps

1. **Done: Editing a `Value` takes four lines every time.** The showcase
   repeated get → local → widget → `if changed` → `set_signal` about a dozen
   times (`example/showcase/gui-core/src/app/sections.rs`). `write` could not
   be used instead: it took `impl Fn`, so the closure could not touch `ui`,
   and it sent on every call even when nothing changed.
   - `Value` and `ValueAtomic` now have `edit` (sends with `set`) and
     `edit_signal` (sends with `set_signal`). They edit a copy, send only when
     it changed, hold no lock while the closure runs, and return the
     closure's result. The counter and the showcase use `edit_signal`.
   - `Diff` and `DiffAtomic` stay. ExpertPI uses them at 48 sites, and some
     of them can't be expressed well with one closure: a widget that edits
     two values at once (value and step size, e.g. focus and `focus_step`),
     or a snapshot that crosses several nested closures (`scanning.rs`).
   - `write` and `write_signal` stay as the clone-free way to edit large
     values in place (a `Vec`, a `HashMap`). They no longer send on every
     call: the closure reports a change through its result (`bool`,
     `egui::Response`, or `InnerResponse`/`Option` wrapping one, via the new
     `Changed` trait), and the value is sent only then. This is breaking
     (`feat!`) for closures that returned another type; nothing in the repo
     or ExpertPI relied on that.
2. **Done: Repaint was off by default.** The showcase server passed
   `update=True` on all 39 runtime sets (`example/showcase/python/run.py`),
   and ExpertPI on 99 of its 255 sets. A set that forgot the flag left the UI
   stale until something else repainted it.
   - The `update` flag is removed from every setter (Python and Rust server)
     and from the wire. The client repaints once per received WebSocket frame
     that changed a state, after applying all of its messages. The server
     already merges consecutive messages into one frame, so a burst of sets
     usually gives one repaint. The first and middle chunks of a split
     transfer don't repaint. `server.update(dt)` stays for explicit or
     delayed repaints.
   - Breaking (`feat!`): callers drop the argument. ExpertPI still has 119
     `update=` arguments to remove. The wire change is covered by the
     unreleased `PROTOCOL_VERSION` 7.
   - Not done: a scoped `hold_repaint` that holds one thread's messages and
     sends them together. It's only worth adding if intermediate frames turn
     out to matter.
3. **Done: The client could not tell that a state changed.** `Data`,
   `Static`, `VecState` and `MapState` could only be re-read each frame, so
   anything derived from them (a downsampled plot, a texture built from
   `Data<u16>`) was recomputed or diffed every frame.
   - Done as a flag instead of a version counter. `Data`, `VecState` and
     `MapState` have `take_changed()`, set after a visible server change and
     cleared only by that call, never by reads.
   - `DataMulti` has `take_changed(key)` (it also reports a removed key) and
     `take_any_changed()`.
   - The flags are atomics with no new lock. Partial transfers and failed
     updates set none.
   - Skipped `Static`. A `version()` counter for several independent readers
     can be added later without breaking anything.
4. **Done: Server-to-client events could be lost.** `ValueTake` held one
   slot, so a second value replaced an untaken one, and `blocking` stalled the
   server thread until a frame called `take()`.
   - `#[state(queue)]` on a `ValueTake` keeps up to 100 untaken values on the
     client, `#[state(queue = N)]` up to N. A full queue drops the oldest and
     reports it once. New `len()` and `clear()`. No wire change.
   - A send with no client stays as it was, by decision: nothing waits, the
     value is dropped, or cached with `cache` on data takes. It is now
     documented on every take `set`.
   - Untaken take values (`ValueTake`, `DataTake`, `DataMultiTake`) are
     dropped once a new connection is accepted; they stay takeable during an
     outage.
   - Found on the way: a non-blocking `DataTake` went straight into the
     unbounded send queue, so a fast producer on a slow link grew memory
     without limit. The data takes now use the `Data`/`Image` scheduler: one
     transfer at a time, acknowledged on arrival, or on take with
     `blocking`. New `wait` parameter and `is_idle()`. No queue on any Data
     state, by decision.
5. **Done: Python `Vec` and `Map` were not Pythonic**
   (`egui_states/structures.py`).
   - Neither had `__len__`, and a missing index or key raised `ValueError`.
     Without `__iter__` and `__contains__`, Python fell back to calling
     `__getitem__` with 0, 1, 2, … until it saw `IndexError`. Checked on a
     real server before the fix: `list(vec)` raised `ValueError`,
     `1 in map` raised `ValueError` even with key 1 present, and `vec[-1]`
     raised `OverflowError`.
   - Both now support `len()`, iteration over a copy (a map yields its
     keys) and `in`. `Map` checks `in` natively without copying, and a key
     of the wrong type is not in it, as for a dict. `Vec` gained `del vec[i]`
     and negative indices.
   - A missing index raises `IndexError`, a missing key `KeyError(key)`.
     Breaking (`feat!`) for code that caught `ValueError`.

## States

| Action | State | Why |
| --- | --- | --- |
| Keep (decided) | `ValueAtomic`, `StaticAtomic` and the `Atomic*` traits, locks and derives | Kept for the fast `get()`, which egui code calls every frame. Measured in a release build: 0.4 ns per `get()` on `ValueAtomic` against 11–13 ns on `Value` (`f32`, `u64`, `bool`, uncontended), and an atomic read never waits behind a server update. Simplified instead (`2c8a4aa`): one safe `Atomic` trait (`to_atomic`/`from_atomic` to a `u64`) and one derive replace the four `unsafe` traits, the `AtomicStatic` derive and the lock types; the never-compiled fallback for targets without 64-bit atomics is gone, and such a target gets a compile error. |
| Keep (decided) | `Diff`, `DiffAtomic` | `edit` covers the single-value case; `Diff` is still needed when one widget edits several values (see gap 1). |
| Remove | `signal_set_to_queue` / `signal_set_to_single` | Duplicates `#[state(queue)]` (`egui_states/structures.py:78`, Rust `server/values.rs`). |
| Done: fixed, kept | `DataMultiTake` | Pending keys could not be listed, so the showcase polled `for key in 0..4`. It now has `take_next()` (the key that waited longest first), `len()` and `clear()`, and its keys no longer wait for each other. |
| Done (`8811c04`) | Header on `Image` | `Data` got a typed header so buffer and description never tear; an image with its frame number or scale has the same problem. `Image<H>` and `ImageMulti<H>` now carry one with their pixels: the set that creates an image (or key) needs it, a change without one keeps it, a split update sends it with its first part, and `set_header` sends it alone after the waiting parts. The client stores it with the texture; egui's texture upload at the end of the frame can show the two one frame apart, which is accepted. `initialize_with_header` gives a placeholder one. Rust server `*_with_header` methods, Python `ImageWithHeader`/`ImageMultiWithHeader`, both code generators, the showcase, and an `image-headers` integration scenario. Also fixed a race in the `images-wire` fixtures that the per-key acks of item 6 exposed (`a34ee4e`). |
| Done | Queue mode for `ValueTake` | Covers gap 4. |
| Done (`ab7b61c`) | Enums with data in `typed` | Only fieldless enums worked (`ObjectType::Enum`); one queued `Signal<Command>` gives ordered commands, where separate signals run concurrently across workers. `#[typed]` now accepts unit, named and tuple variants (`ObjectType::DataEnum`, no wire change; fieldless enums keep their hash and `IntEnum`); empty `{}`/`()` variants, discriminants, generics and direct recursion are compile errors. `InitialValue` and both generators support them, so they work in every value, static, signal, take, collection, header and transfer metadata. Python: unit variants are constants, named variants frozen dataclasses, tuple variants tuples whose equality includes the variant; a generated `enums.pyi` stub types them for `ty`. `#[derive(Atomic)]` stays fieldless; the `Atomic` docs show a hand-written one. Showcase "enums with data" panel and a `data-enums` integration scenario. |
| Maybe | Request/response | Picking a path on the server's machine from a remote UI needs a directory listing; today that is a `Signal` plus a `Static` with nothing linking them. |

Everything else is worth keeping, including `Static`, `Signal`, the `Data`
family, `ImageMulti` and the transfers.

## Smaller API issues

- **Done: Closure bounds differed:** `Fn` on `Value::read` and client
  `Data::read`, `FnMut` on `VecState::read`, `FnOnce` on server `Data::read`.
  Every method that calls its closure once (`read*`, `write`,
  `write_signal`, server `Data::read_bytes`) now takes `FnOnce`. `for_each`
  keeps `FnMut`, and stored callbacks keep `Fn + Send + Sync`.
- **Done: `DataMulti` lacked key listing:** it had no `len`, `contains` or
  `indices` on the client, the Rust server or Python, while `ImageMulti` had
  all three. It now has them (Python: `indices()`, `len()`, `in`). Python
  `in` on `ImageMulti` raised `OverflowError` for an index outside the u32
  range; it now returns `False`, as `DataMulti` does.
- **`VecState` is thin:** the wire has set-all, set-item, append and remove
  only, so there is no insert-at-index or extend, and the client has no `len`.
- **Naming:**
  - `Signal::set` reads oddly for an event; `emit` fits better.
  - Collections are `VecState`/`MapState` in Rust and `Vec`/`Map` in Python.
- **Duplicates:** Rust `set_with_signal` (third spelling next to `set` and
  `set_signal`), Python `StatesBase.update_ui` (same as `server.update`,
  `egui_states/server.py:114`), `ClientBuilder::addr` (same as `host`).
- **Python `connect` returns `None`:** returning the callback would let it
  work as a decorator. The Rust server returns a `CallbackHandle`.
- **Partly done: Rust server calls took up to four positional bools.**
  Removing `update` (gap 2) left at most two, but item 6 added `wait` to
  the data takes: `take.set(data, blocking, cache, wait)`. Kept positional
  by decision; an options struct can come later.
- **Done: The keys of a multi state shared one transfer stream.**
  `ImageMulti`, `DataMulti` and `DataMultiTake` now acknowledge and wait
  per key (new client message `AckKey`), so a slow or blocking key does
  not hold up the others. `is_index_idle(index)` in Rust, `is_idle(index)`
  in Python.
- **`Image` texture filtering is fixed to nearest:** there is no texture
  option, and the client cannot read pixel values back.
- **`Data` is flat:** Python `get()` returns a 1-D array, so the shape has to
  go in the header.

## Beyond states

- **Done: Auto-reconnect on the client.** `Client::connect()` had to be called
  again after every failure.
  - `ClientBuilder::reconnect(Reconnect)` retries `Unreachable`, `Timeout`,
    `ConnectionLost` and the new `ServerStopped` with a doubling wait
    (0.5 s up to 5 s), and `Busy` every 5 s. Other reasons still wait for
    `connect()`. The new state is `ConnectionState::Reconnecting`.
  - The server's stop now closes with 4007 (`ServerStopped`), so that a
    retry doesn't undo `disconnect_client` (4006, `ClosedByServer`).
  - `disconnect()` now also cancels an attempt that is still opening.
  - Not done: the client doesn't notice a silent server. The server pings
    every 5 s, but the client has no read timeout, so a connection the
    network dropped stays `Connected` until the OS reports it.
- **Logging.** `server.logging` is documented as client messages
  (`egui_states/logging.py:1`, `crates/egui-states/src/server/logging.rs:1`),
  but it carries the server's own diagnostics from
  `server_core/net/session.rs`. Route it to Python's `logging` and Rust's
  `log` and drop the per-level `add_logger` API. Client-side errors already
  arrive through `on_client_message`.
- **Stale docs.**
  - `docs/architecture.md:346` and `:388` still mention the C API removed in
    #79 (`95eedfc`).
  - The showcase UI still labels the collections `ValueVec`/`ValueMap`
    (`example/showcase/gui-core/src/app/collections.rs:5`, `:36`).
- **Maybe:** dumping and restoring all `Value`s as a dict, for persisting
  settings between runs.

## Suggested order

1. ~~`Value::edit`~~ (done; `Diff` kept, `write` and `write_signal` send
   only reported changes).
2. ~~Repaint on by default~~ (done: no `update` flag, one repaint per frame).
3. ~~Python `Vec`/`Map`~~ (done: list/dict protocol, negative indices,
   `IndexError`/`KeyError`).
4. ~~Client auto-reconnect~~ (done: `ClientBuilder::reconnect`,
   `ServerStopped`).
5. ~~Version counter on client states~~ (done as `take_changed()` flags;
   `Static` skipped).
6. ~~Queue mode for `ValueTake`; decide on `DataMultiTake`~~ (done: client
   queue, per-key acks, takes on the scheduler, `take_next()`).
7. ~~Remove the atomic variants~~ (kept for the fast `get()`, simplified to
   one safe `Atomic` trait instead).
8. ~~`Image` header~~ (done: `Image<H>`, `ImageMulti<H>`, header with the
   first part of a split update), ~~enums with data~~ (done, `ab7b61c`:
   unit, named and tuple variants; Python constants, dataclasses and tuples).
