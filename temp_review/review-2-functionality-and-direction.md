# egui-states review 2: functionality, features, and overall assessment

Reviewed at commit `e3643b5` (main, 2026-10-18), workspace version 0.16.0.
Review 1 (`review-1-bugs-performance-testing.md`) covers defects and test
gaps; this document is about what the project does, what it should do next,
what it could stop doing, and how it holds together as a whole.

---

## 1. What the project is, in one paragraph

A typed state-synchronization layer between an egui UI (native or WASM) and a
backend written in Rust, Python, or, experimentally, C. A Rust struct is the
single source of truth; a derive produces the client handles, and build-time
generators produce matching server bindings. Fourteen state kinds cover
scalars, collections, numeric buffers, images, and one-shot transfers. The
wire is postcard over one WebSocket, with name-hashed ids, per-value type
hashes, acknowledgement-based backpressure for large transfers, and an
optional static web server so a browser build can be served from the same
port. Exactly one client is connected at a time; a second connection replaces
the first.

## 2. Overall opinion

This is a well-engineered niche library. The core idea (derive once, generate
the other side, hash everything so mismatches are caught) is right, and the
execution is careful: the protocol has golden tests for every id and hash,
the synchronization model is documented in `docs/architecture.md` with
precise semantics for coalescing and previous values, the Python API is typed
and documented, the examples are real and tested, and the test culture is
well above average for a one-maintainer project (see review 1). The codebase
reads like it was written by someone who has been bitten by every race it
guards against.

The main risks are not quality but shape:

- Breadth has grown faster than abstraction. Fourteen state kinds, three
  server languages, and five hand-written transfer state machines mean every
  new feature is implemented up to six times. The duplicated code is where
  several of the bugs in review 1 were found.
- Several design decisions optimize for a single local client (one
  connection, no reconnect, no TLS, IPv4 literal addresses).
  That is a fine product definition, but the browser client and the
  `serve_web` feature pull in the opposite direction, toward "a dashboard
  other people open in a browser", and the current connectivity layer cannot
  serve that.

If I had to pick one strategic question for the next release it would be:
is egui-states a *local control-panel transport* (one trusted client, tight
coupling, maximal throughput) or a *UI state bus* (several viewers,
reconnects, browsers across a network)? Most of the proposals below follow
from choosing, and the current code sits between the two.

---

## 3. Strengths worth preserving

- Single schema, generated bindings, compile-time checks in Rust and
  type-descriptor checks in Python. Changing the state struct and rebuilding
  really is the whole workflow.
- Protocol hygiene: stable ids, per-type hashes, an explicit
  `PROTOCOL_VERSION` policy in `AGENT.md`, golden tests, no serde attributes
  that can silently change the wire shape (the `typed` macro rejects them).
- Backpressure that actually works: large `Data`/`Image` transfers are
  chunked and acknowledged, so a fast producer cannot exhaust the client.
- Thoughtful callback semantics (single/queue modes, `connect_previous`
  returning the last *delivered* value, one state never runs concurrently,
  panics and handler failures contained).
- `serve_web` is a genuinely useful convenience: one port, no extra server,
  correct ETag/304/HEAD handling, no path traversal.
- Documentation quality: README, architecture notes, rustdoc on everything
  public, Google-style Python docstrings, generated code that is readable.

---

## 4. Features I would add

Ordered by the value I think they deliver, with the cost in parentheses.

### 4.1 Connection by URL, hostname, `wss://`, and browser-derived defaults (medium)

`ClientBuilder::addr(Ipv4Addr)` plus a port is the only way to connect. A
WASM client served from `serve_web` over anything but plain `http://localhost`
cannot connect at all: browsers block `ws://` from an `https://` page, and the
client cannot be pointed at a hostname. Add `ClientBuilder::url(&str)` (host,
port, path, `ws`/`wss`), IPv6, and on wasm a default of "same origin as the
page, `/ws`". TLS on the native side via `tokio-tungstenite`'s `rustls`
feature is a few lines; on the server side TLS is more work but can also be
left to a reverse proxy once hostnames and `wss` are accepted. This is the
single feature that turns `serve_web` from a demo convenience into something
deployable.

### 4.2 Automatic reconnect with backoff (medium)

Today a dropped connection stays dropped until the application calls
`connect()` again. Add `ClientBuilder::reconnect(policy)` (exponential backoff,
max attempts).

### 4.3 Multiple simultaneous clients (large, strategic)

The "second client replaces the first" rule is the biggest functional limit.
Two realistic asks: a second read-only viewer (a colleague opens the browser
page), and a native control panel plus a browser dashboard at once. A full
multi-writer design needs per-connection acknowledgement tracking for every
transfer state and a policy for conflicting `Value` writes. A pragmatic first
step is *observer* connections: they receive every broadcast message, never
send `Value`/`Signal`, and do not participate in backpressure (chunked
transfers are published to observers after the primary client acknowledges
them, or dropped for observers under pressure). That reuses the existing
ack machinery and already covers the dashboard case.

### 4.4 Async-first Rust server API (medium)

`StateServer::start` insists on owning a thread and a runtime and must not be
called from inside Tokio. Rust backends are increasingly async; offer
`StateServer::start_on(&tokio::runtime::Handle)` or `serve(listener).await`,
async `set` variants that `await` acknowledgements instead of blocking a
thread, and `Stream`-based callbacks (`value.changes() -> impl Stream<Item =
T>`). The existing synchronous API can stay as a wrapper.

### 4.5 Python asyncio bridge and context managers (small)

`await value.changed()`, `async for v in value.changes()`,
`await server.wait_for_client(timeout)`, and `with StatesServer(...) as
server:` for deterministic shutdown. The callback-thread model is fine, but
most Python applications that would use this already run an event loop.

### 4.6 Client-side change notifications (small)

The UI polls `get()` every frame and the take states are polled with `take()`.
A per-handle `changed()` flag that is set by the receive loop and cleared when
read (or a frame counter of the last update) lets applications redraw only
what changed, run side effects once, and avoids the per-frame clone noted in
review 1. On the server side the same information already exists.

### 4.7 A "resend" request (small)

Add a client to server `Resync(id)` request so a client can ask for a state
again after dropping a malformed update, instead of waiting for a reconnect.

### 4.8 Configurable limits (small)

`VALUE_MAX_SIZE` (1 MB), `MSG_SIZE_THRESHOLD` (10 MB), handshake timeout, and
the batching ceiling are compile-time constants with a `TODO: make these
constants configurable`. Put them on `ServerOptions`/`ClientBuilder`, send the
server's limits in the handshake reply so the client can enforce the same
numbers, and expose them to the tests so the large-transfer tests can shrink.

### 4.9 Enforce the layout hash by default (tiny)

The machinery exists (`get_version_hash`, `VERSION_HASH`), but it is opt-in,
so a stale binding connects and then produces a stream of per-value type
mismatch errors. Make the generated servers pass their hash by default and
let the application override `version` when it deliberately wants to accept
several layouts.

### 4.10 Observability (small)

A `tracing` (or `log`) feature on both sides replacing the debug-only
`println!`; per-state counters (messages, bytes, pending acknowledgements,
coalesced changes); and a `StateServer::stats()` snapshot. For a
synchronization library, "why is my value not updating" is the most common
support question, and today the answer requires a debugger.

### 4.11 Shape-aware numeric data and bounded append (small)

`Data<T>` is a flat 1-D buffer. NumPy users will pass 2-D arrays (the Python
binding already flattens any C-contiguous buffer silently). Carrying an
optional shape in the header makes `get()` return the right array on both
sides. A `Data::push_bounded(value, capacity)` ring-buffer append covers the
very common "scrolling plot of the last N samples" case without resending the
whole buffer or letting it grow forever.

### 4.12 An in-process transport (medium)

A loopback transport (channels instead of sockets) would let tests run
without ports, let a Rust application host the server and client in one
process (useful for prototyping and for tools that embed both), and make the
WASM client testable in native unit tests. Most of the code is already
transport-agnostic behind `MessageSender`/`MessagesParser`.

### 4.13 Generated documentation and an inspector (small, nice to have)

The generators have the complete schema; emitting a Markdown table of states,
types, and initial values is trivial and useful as living documentation. A
step further is a `Describe` message in the handshake reply so a generic
inspector client (an egui app that knows no schema) can connect to any
server, list states, and show live values. That also gives you a debugging
tool for the support question above.

### 4.14 egui binding helpers (small)

Thin helpers that pair a handle with a widget and apply the `Diff` pattern:
`value.show(ui, |ui, v| ui.add(DragValue::new(v)))` sends only if the widget
reported a change. The showcase repeats that boilerplate for every widget;
users will too.

---

## 5. Things I would remove, hide, or merge

None of these are broken. Each adds API surface, generated-code shapes, or
maintenance cost out of proportion to what it buys, and together they make
the library harder to learn than it needs to be.

### 5.1 `ValueAtomic`, `StaticAtomic`, `Atomic`/`AtomicStatic` derives, and the lock traits

About 400 lines of `unsafe` public traits (`Atomic`, `AtomicStatic`,
`AtomicLock`, `AtomicLockStatic`, `UpdateLock`, `FallbackLock`, eight lock
newtypes, two derive macros) exist so that small `Copy` values can be read
without an `RwLock`. An uncontended `parking_lot::RwLock` read is a single
atomic operation; the UI thread reads once per frame; and `UpdateLock` holds a
`Mutex` across the send anyway. Users must choose between `Value<f64>` and
`ValueAtomic<f64>` with no observable difference, and the choice is baked into
the layout hash (`ATOMIC_HASH_ID`), so switching later breaks compatibility.
Recommendation: deprecate the public atomic kinds, keep `Value<T>` and
`Static<T>`, and if a lock-free read ever matters, specialize internally on
`T: Copy` without exposing it.

### 5.2 `Queue` / `NoQueue` as a type parameter on client types

`Value<T, Q>` and `Signal<T, Q>` carry a *server* callback policy in the
*client's* type system, solely so the generators can emit `queue = true`. The
server already has `signal_set_to_queue()` at runtime. Replace the type
parameter with a derive attribute (`#[state(queue)]`) that the generators
read, or make queue mode a server-side default per state kind. This removes a
generic from every public client type and from every generated signature.

### 5.3 The take family: `ValueTake`, `DataTake`, `DataMultiTake`, `blocking`, `cache`

Three state kinds, two flags, and roughly 2.5k lines implement "server sends
something the client consumes once". `ValueTake<()>` is a server-to-client
signal in disguise; `cache = true` makes a one-shot value behave like a
`Static` for new clients; `blocking` is a backpressure knob that would be
useful on *every* server-to-client state. I would collapse this to one concept
(`ServerSignal<T>` for small values, consumed from a client-side queue) and
make backpressure a property of the transfer layer, not of the state kind.
`DataMultiTake` in particular is hard to justify; its only distinct
capability is per-key take state, which a `HashMap<u32, Vec<T>>` payload on a
`ServerSignal` provides.

### 5.4 `InitialValue` / `InitValue` and the literal renderers

To embed initial values in generated code, every custom type must also derive
`InitialValue`, and the generators carry two renderers (Rust literals, Python
literals) plus C writer calls, each with special cases for NaN, infinities,
escaping, keyword renaming, canonical map order, and `i32::MIN`. All of that
disappears if the generators serialize the initial value with postcard once
and embed the bytes: Rust bindings get `&[u8]` constants decoded at startup,
Python bindings get `bytes`, C gets a byte array. The schema stays
human-readable through the type table; a comment can carry a `Debug`
rendering. This removes a derive users must remember, around 600 lines of
rendering code, and an entire class of bugs.

### 5.5 Two channels for client diagnostics

`LoggingSignal` (four levels, four logger lists) and `on_client_message`
both carry "text from the client". One channel with a level, surfaced through
`log`/`tracing` on the server, is simpler and plays with existing logging
setups.

### 5.6 The experimental C ABI, unless a consumer is committed

`crates/egui-states-c` plus `src/c_abi` are about 9k lines including tests,
roughly the size of the whole client. It is well built (type-checked
readers/writers, panic containment, golden header), but it has no consumer
beyond the counter example, and every new state kind now needs a fourth
implementation. If the goal is C++ or C# servers, I would either commit (ship
a header-only C++ wrapper and a NuGet package, and let them drive the ABI) or
park the crate in a separate repository until there is demand. Keeping it
"experimental" in-tree means paying the maintenance cost without the users.

### 5.7 Minor surface trims

- `Image::set_all` fill and the `force` flag on `update` are niche; fine to
  keep but should not be in the first page of the docs.
- `StaticAtomic<[f32; 2]>` and `(f32, f32)` packed-atomic specializations go
  away with 5.1.
- `ValueTakeEmpty`/`SignalEmpty` Python classes exist only because `()` is
  awkward to spell; with 5.3 they merge into one.
- The release workflow now builds the wheels, so the `scripts/build-*`
  helpers largely duplicate it; keep them only for local wheel builds.

---

## 6. Design notes and questions for the maintainer

- Single-client semantics are the right simplifying assumption for a control
  panel and the wrong one for a dashboard. Decide which, and say so in the
  README. The current README reads as both.
- Name-hash ids: 64-bit SHA-256 prefixes keyed by the dotted path, collisions
  checked at registration. Solid. The only wrinkle is that ids are on the
  wire, so the hash function is frozen; that is fine as long as it is written
  down (it is, implicitly, by the golden test).
- The `#[doc(hidden)] pub mod c_abi` boundary is a public API that the
  `egui_states` crate's semver does not acknowledge. A separate crate makes
  the contract honest.
- The repository is developed in very large PRs (the last three add about
  20k lines). That is workable for one author but makes review by anyone else
  impossible; smaller, feature-scoped PRs, now that CI runs on every pull
  request, would lower the bus factor.
- Documentation gaps: a wire-format reference (frame layout, header enums,
  ack rules) separate from the architecture overview; a troubleshooting page
  (value not updating because a callback is blocked, port already in use,
  protocol version mismatch); and an explicit
  security statement (no TLS, token in clear text, trust the network).
- The Python package has no published API documentation site; the docstrings
  are Sphinx-ready, so this is a day of work.

---

## 7. Verdict

Keep the core (schema, generators, protocol hygiene, acknowledgement-based
transfers, tests). Fix the remaining bugs from review 1 (B15); CI is
already in place. Then spend the next release on *less*: unify the transfer
state machines, retire the atomic variants and the `Queue` type parameter,
collapse the take family, and embed initial values as bytes. With that done,
the strategic features (URL/`wss` connectivity, reconnect,
observers, async and asyncio APIs) become additions to one
implementation instead of five, and the project is in a good position to grow
from "my egui tool talks to my Python script" into a dependable UI state bus.
