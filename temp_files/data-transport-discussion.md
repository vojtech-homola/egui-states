# Data transfer channel: design discussion

Date: 2026-11-10. Branch: `data-transport-channels`. Nothing in the repository
was changed during this discussion.

---

## User

I would like to introduce new feature to this project:
I want to have possibility to transfer data between server and client, but this
transfer is not directly related to the GUI, it is simple data transfer e.g.
transfer files. Data can be huge, so I do not want to use same channel as is
used for sync gui states. I just want to start discussion, do not edit any code.
Do you have any idea how to implement it and how API should look like?

## Claude

I haven't changed anything in the repo. This proposal is based on how the
server and client work today.

### Why the current connection can't carry bulk data

Even apart from your preference, three things in the current design rule it out:

- **Head-of-line blocking.** Every outgoing message goes through one queue and
  one writer task (`crates/egui-states/src/server_core/core.rs:463`,
  `DataReceiver` at `:592`), which batches messages up to 10 MB. On a 20 Mbit/s
  link, one 10 MB batch holds back every state update for about 4 s. Cutting the
  data into smaller chunks on the same socket only partly helps, because the
  operating system's send buffer for the socket (often several MB) still sits
  ahead of each state update.
- **Memory.** `MessageSender` (`sender.rs:9`) is unbounded. Pushing a 20 GB file
  through it would mean holding the whole file in memory. Bulk data needs a
  sender that waits until the receiver is ready for more.
- **Heartbeat.** `docs/architecture.md:123` already says the heartbeat timeout
  must be longer than the longest transfer. Putting files on the main
  connection would force very long timeouts, so a dead client would take much
  longer to detect.

### Transport: a second WebSocket on the same port and URL

| Option | Verdict |
|---|---|
| A raw TCP connection on a second port | Fastest, but it breaks browser (WASM) clients, the Trunk proxy, and setups where only one port is reachable (SSH tunnels, Docker) |
| Plain HTTP requests on the same port | Convenient for browser downloads, but native clients would need an HTTP client library and the server would need to parse uploads |
| **A second WebSocket on the same port** | **Recommended** |

How it would work:

- **Same URL; the first message tells the two apart.** The main connection
  opens with `Handshake`; the data connection opens with something like
  `DataHello { protocol, ticket }`. The web dispatch (`web.rs:288`) and the
  Trunk proxy, which forwards only `/ws`, then need no changes.
- **A session ticket instead of the token.** Once the main handshake succeeds,
  the server sends a random 128-bit ticket over the main connection, and the
  data connection must present it (`tokens_match` at `core.rs:380` can do the
  comparison). This is better than repeating the token, for two reasons:
  - The token is optional and off by default, so without a ticket any local
    program could attach a data connection and push uploads into the current
    session.
  - The ticket ties the data connection to one session. When a new client
    replaces the old one, the old data connection becomes invalid
    automatically, which fits the one-client model.
- **Lifetime.**
  - When the main session ends, the data connection closes and all transfers
    fail with `Disconnected`.
  - If only the data connection drops, the client reopens it with the same
    ticket; the transfers that were running fail.
- **Its own heartbeat.** Because data travels in small chunks and the receiver
  sends frames back regularly, its timeout can stay short too.
- **No cost when unused.** The client opens the data connection only if the
  state tree declares at least one transfer endpoint.

### Inside the data connection

A small set of frames, similar in spirit to HTTP/2 streams:

```
Open   { xfer, endpoint, meta, size: Option<u64> }   sender → receiver
Accept { xfer, credit } | Reject { xfer, reason }    receiver → sender
Chunk  { xfer, <raw bytes> }                         sender → receiver
Credit { xfer, bytes }                               receiver → sender
End    { xfer } | Cancel { xfer, reason }            either side
```

- **Chunks of about 256 KiB to 1 MiB, taken from active transfers in turn**, so
  a small upload isn't stuck behind a 50 GB download. Uploads and downloads can
  run at the same time.
- **Credit-based flow control per transfer.** The receiver grants a window of
  roughly 4–8 MiB (configurable; it should cover bandwidth × round-trip time),
  and the sender reads the next chunk from disk only when it has credit. Memory
  use stays bounded whatever the file size.
- **`Accept` is simply the first credit grant.** Nothing flows until the
  receiver decides where the data should go, so the receiver gets to approve
  every push without an extra mechanism.

### API sketch

I'd declare transfer endpoints in the state tree like the other state types.
That gives names and ids, typed metadata through the existing type machinery,
generated Python bindings, and the layout-hash check. The bytes themselves
would not be synchronized state; the handle just owns the endpoint.

```rust
#[derive(egui_states::State)]
pub struct AppState {
    pub results: Transfer<FileInfo>, // FileInfo: any type usable in Value<T>
}
```

Python server:

```python
files = server.states.results

# server → client
t = files.send_file(Path("out/result.h5"), FileInfo(name="result.h5"))
t = files.send_bytes(payload, FileInfo(...))
t = files.send_iter(chunks(), FileInfo(...), size=None)  # generator, size unknown
t.progress()  # (done, total | None)
t.wait(timeout=60)  # raises TransferError on cancel or disconnect
t.cancel()


# client → server
def on_incoming(incoming: Incoming[FileInfo]) -> None:
    if incoming.size is not None and incoming.size > LIMIT:
        incoming.reject("too large")
        return
    incoming.accept_file(upload_dir / safe_name(incoming.meta.name))


files.on_incoming(on_incoming)
```

Rust client, inside the egui update loop:

```rust
if let Some(incoming) = state.results.poll_incoming() {
    // e.g. after a save dialog
    self.download = Some(incoming.accept_file(path)); // or accept_memory() / accept_stream()
}
if let Some(t) = &self.download {
    ui.add(egui::ProgressBar::new(t.progress().fraction()));
    // t.status(): Running | Done | Failed(err) | Cancelled
}
let upload = state.results.send_file(path, FileInfo { /* … */ });
```

Behaviour I'd define explicitly:

- **Where received data goes:**
  - `accept_file(path)` writes to a temporary file next to `path` and renames
    it when the transfer completes, so a failed transfer never leaves half a
    file.
  - `accept_memory()` returns the data as bytes, subject to a size limit.
  - `accept_stream()` hands over chunks as they arrive, for processing on the
    fly.
- **The receiver chooses the path.** A filename in the metadata is only a hint;
  never write to a path the sender supplied (path traversal).
- **Repaints.** Progress requests a repaint, limited to about 10 per second.
- **Unanswered offers.** If the receiver never accepts, no data flows and the
  sender waits. An optional `auto_accept` (into memory, up to N bytes) covers
  simple cases.
- **Python threads.** Disk reads and writes happen in Rust, outside the GIL.
  `wait()` releases the GIL, and callbacks run on the existing callback
  workers.

### Things to settle

- **No ordering with state updates.** The two connections are independent. If
  the server sets a `Static` to "ready" and then sends the file, the client can
  see either one first. Completion has to be reported on the transfer handle or
  callback, and the docs must say so.
- **Sharing a slow link.** The bulk stream also fills the network's queues,
  which adds some delay to the main connection: less than head-of-line
  blocking, but noticeable with large images. Given how much you care about
  latency, an optional rate cap such as `max_rate` would be cheap to add.
- **Resuming.** For very large files over unreliable remote links, an
  `Open { offset }` plus a stable transfer key would allow resuming after a
  reconnect. I'd leave room for it in the frame format but implement it later.
- **Browser clients.** The WebSocket works in a browser, but there's no
  filesystem: received data would go to memory and then a browser download,
  and uploads would come from the browser's File API. Native-only for the first
  version seems reasonable.
- **C ABI.** Leave it out at first; it can be added later as opt-in.
- **Protocol version.** The ticket message and the new opening message break
  wire compatibility, so `PROTOCOL_VERSION` becomes the released value + 1.

### Questions for you

1. Which directions do you need: server → client, client → server, or both?
2. What sizes and networks do you expect? A LAN versus a remote machine over an
   SSH tunnel decides the window size, whether a rate cap is needed, and
   whether resuming matters.
3. Should endpoints be declared in the state tree (typed, with generated
   bindings), or would you prefer a looser API on the server and client objects
   with string names and untyped metadata?
4. Do browser clients need this in the first version?
5. Do you need several transfers at once, or is a one-at-a-time queue enough to
   start? The frame format supports both; only the scheduling differs.

---

## User

1. I need both directions
2. Size can be up to tens of the GB, I expect stable 1gb connection
3. I like to have declared it in state tree, but I also would like to keep
   implementation separate from rest of the states sync as possible
4. Yes
5. Having queue is fine

## Claude

Thanks, that settles the main design. Your answers change four things: both
peers need the same send and receive engine, tens of GB means every transfer
must stream with bounded memory, browser support brings in some web-specific
questions, and keeping it apart from state sync decides the module layout. I'm
still only discussing; nothing in the repo has changed.

### What 1 Gbit/s and tens of GB mean

At 1 Gbit/s you get about 110–118 MB/s of payload, so 10 GB takes about 1.5 min
and 50 GB about 7 min. Raw speed tuning matters less than reliability and
feedback over several minutes:

- **Memory stays bounded.** Each side holds at most the credit window plus one
  chunk per transfer (1 MiB chunks, ~8 MiB window), whatever the file size. On a
  LAN the window is far more than the round trip needs; its real job is to
  absorb disk pauses.
- **The slower disk sets the pace.** A hard disk at ~150 MB/s barely keeps up.
  With credit-based flow control, a slow disk on the receiver slows the sender
  down instead of filling memory.
- **Fail early.** The size is known when a transfer is offered, so accepting it
  should check free disk space. Otherwise you find out at 95%.
- **Optional end-to-end check.** A SHA-256 computed while streaming and
  verified at the end needs no new dependency, because `sha2` is already used.
  With CPU SHA support it runs above 1 GB/s, and in software it's still faster
  than line rate. I'd make it opt-in per transfer.
- **Resuming.** On a stable link I'd leave it out. I'd still reserve an
  `offset` field in the offer now, so it can be added later without a protocol
  change.
- **Shared link.** When an image stream and a transfer run at the same time,
  TCP gives each roughly half the link, so images still get ~500 Mbit/s. A rate
  cap can stay optional.

### Keeping it separate from state sync

Because both directions are needed, the engine is symmetric: the server and the
client run the same code for framing, queues, credit accounting, sources and
sinks. Only how the connection is set up differs. All of it can live in its own
module:

```
crates/egui-states/src/transfer/
  protocol.rs   frames, hello message, ticket
  engine.rs     per-direction queue, credit accounting, cancellation
  source.rs     file / bytes / iterator / browser File
  sink.rs       file (temp + rename) / memory / stream / browser download
  server.rs     accepts the data connection, owns the ticket
  client.rs     dials the data connection (native + wasm)
  python.rs     separate PyO3 object, not new methods on StateServerCore
egui_states/transfer.py
```

The engine runs over any WebSocket stream, so it can be tested with two
in-memory endpoints without starting a state server. The existing code is
touched in only three places:

1. **State tree.** A new `transfer::<M>(name)` method on `StatesCreator`
   (`crates/egui-states/src/client/states_creator.rs:49`), plus the matching
   method in the build-script creator and the server's add method. The id and
   type go into the layout hash like any other state. The handle registers in
   its own `TransferRegistry`, not `StatesList` or `ValuesList`, so no sync or
   acknowledgement code ever sees it.
2. **Accepting connections.** After the WebSocket upgrade, so the browser
   Origin check still applies, the first message decides the path. `Handshake`
   goes to the existing code, and a new `TransferHello { protocol, ticket }`
   hands the socket to the transfer service. This means one new `ClientHeader`
   variant. Telling them apart by the first message works through any proxy,
   including Trunk's `/ws` forwarding.
3. **Session lifecycle.** When a client is accepted, the server sends one new
   `ServerHeader` message carrying the ticket before the initial sync, and it
   ends the transfer session on disconnect. On the client, the message handler
   passes the ticket to the transfer client, which then opens the data
   connection. If the state tree has no transfer endpoint, the server sends no
   ticket and the service never starts.

The transfer service should also have its own callback thread for
`on_incoming` and `on_done`, so a slow upload handler can't delay state
callbacks.

### Queue behaviour

- **One queue per direction.** Uploads and downloads run at the same time;
  within one direction, transfers go one at a time.
- **Offers go out immediately.** The receiver sees every queued file at once
  and can, for example, show a list.
- **Only accepted transfers send data**, starting with the earliest accepted
  one in queue order. A save dialog the user hasn't answered doesn't hold up
  the files behind it.
- **Control frames go ahead of data chunks.** Accept, Credit and Cancel jump
  the outgoing queue. Otherwise, with an upload and a download running
  together, credit for one direction would wait behind up to 8 MiB of chunks
  going the other way, and both transfers would stall.
- **Big files hold up small ones.** A 50 GB transfer delays smaller ones in the
  same direction for ~7 min. You said that's acceptable, and since frames carry
  a transfer id, interleaving can be added later without changing the
  protocol.
- **Disconnects.** When the client disconnects, every queued and running
  transfer fails with `Disconnected`.

### Browser support

**Uploads are easy.** `File.slice()` reads lazily from disk in every browser,
and the slices can be sent as Blobs, so even a 50 GB upload never passes
through wasm memory. Two catches:

- As far as I know, eframe reads a file dropped onto the page fully into memory
  (`DroppedFile::bytes`). Large uploads therefore need a file picker that
  returns a `web_sys::File`, such as rfd's `FileHandle::inner()` on wasm.
- `ws_stream_wasm` can only send a `Vec<u8>`. That would mean copying ~115 MB/s
  through wasm memory on the UI thread. The browser side of the data connection
  should use `web_sys::WebSocket` directly and send Blobs.

**Downloads are the hard part.** wasm32 memory is capped at 4 GiB, often less
in practice, so receiving into memory needs a limit of around 1 GiB. For tens
of GB the options are:

| Option | Browsers | Notes |
|---|---|---|
| **HTTP download from the server** (one-time URL, `Content-Disposition: attachment`) | All | The browser's download manager writes to disk: no wasm memory, no storage quota. The server already handles HTTP requests in `web.rs`. Progress is reported by the server. |
| File System Access API (`showSaveFilePicker`) | Chromium only | Streams into a file the user picks; the app controls progress. |
| Write to OPFS (the browser's private storage), then download | All | Uses twice the disk space, and storage quotas may be smaller than tens of GB. |

I'd use the HTTP download as the default for large browser downloads and add
the save picker for Chromium later. It still goes through the same queue; only
the delivery differs, with TCP providing the backpressure instead of credits.
It has two costs:

- **A fourth touch point.** The server must look at plain HTTP requests even
  when `start_web` is off, and today it only does so when a page is being
  served.
- **A `trunk serve` wrinkle.** Trunk forwards only `/ws`, so during development
  the download URL has to point at the server's port directly.

An HTTP download also needs a file name, so the offer should carry `size` and
`file_name` as built-in fields, with the typed `M` for application data
(defaulting to `()`).

### Updated API sketch

```rust
#[derive(egui_states::State)]
pub struct AppState {
    pub datasets: Transfer<DatasetInfo>, // M defaults to ()
}
```

Python server:

```python
ds = server.states.datasets

# server → client
h = ds.send_file("/data/run42.h5", DatasetInfo(run=42), verify=True)  # file_name taken from the path
h.status()  # QUEUED / OFFERED / RUNNING / DONE / FAILED / CANCELLED / REJECTED
h.progress()  # (done_bytes, total_bytes | None)
h.wait()  # releases the GIL, raises TransferError
h.cancel()
ds.send_bytes(payload, meta, file_name="x.bin")
ds.send_iter(chunks(), meta, file_name="x.bin", size=None)


# client → server
def on_incoming(incoming: Incoming[DatasetInfo]) -> None:
    h = incoming.accept_file(UPLOADS / safe_name(incoming.file_name))
    h.on_done(lambda result: process(result))


ds.on_incoming(on_incoming)
```

Rust client:

```rust
// sending
#[cfg(not(target_arch = "wasm32"))]
let h = state.datasets.send_file(&path, meta);
#[cfg(target_arch = "wasm32")]
let h = state.datasets.send_web_file(file, meta); // web_sys::File

// receiving, inside update()
while let Some(incoming) = state.datasets.poll_incoming() {
    // native: accept_file(path) | accept_memory(limit) | accept_stream(f)
    // web:    accept_download()  | accept_memory(limit)
    self.transfers.push(incoming.accept_file(target));
}
for h in &self.transfers {
    ui.add(egui::ProgressBar::new(h.progress().fraction()));
}
```

The Rust server would follow the Python API, and one `TransferHandle` type
covers both directions.

### Still to decide

1. **Large browser downloads:** HTTP download (my recommendation), the
   Chromium-only save picker, or both?
2. **Sending with no client connected:** fail immediately (my recommendation,
   simpler), or wait in the queue for the next client?
3. **Integrity check:** opt-in per transfer (my recommendation), or always on?
4. **Name:** `Transfer<M>`, or something like `FileChannel<M>`?

Once these are settled, I can write it up as a design doc in `docs/` before
implementing anything.
