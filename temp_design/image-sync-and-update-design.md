# Image synchronization and `update` design — conversation notes

Saved 2026-11-16, branch `improve-state-transport` (at `67fc50b`).
A record of a design conversation; nothing here is implemented yet.

Contents:

1. [How images are synchronized](#1-how-images-are-synchronized)
2. [Inconsistency between `set` and `update`](#2-inconsistency-between-set-and-update)
3. [Suggested API improvement](#3-suggested-api-improvement)

---

## 1. How images are synchronized

> **Question:** Short summary, ideally with simple graphics: how are images
> synchronized, how is the data transported?

### 1.1 The path from the server to the GPU

```
 Python / Rust server                      server_core                           Native/WASM client
 ─────────────────────                     ───────────                           ──────────────────
 img.set_image(np.ndarray)  ──►  validate (≤65 536 px per side)
   gray / gray+α / RGB / RGBA    keep a local RGBA copy (used to resync)
                                 pack into messages: header + raw pixels
                                          │
                                          ▼
                                 MessageSender (unbounded mpsc)
                                          │
                                          ▼
                                 writer task / DataReceiver
                                 • small msgs (≤64 KiB) merged into one frame
                                   (up to 10 msgs / 10 MB)
                                 • big msgs / chunks sent alone
                                          │
                              ══════ WebSocket binary frame ══════►  decode ServerHeader
                                                                     convert pixels → RGBA (to_rgba)
                                                                     build egui ColorImage
                                                                     load_texture / set_partial
                                          ◄═════════ Ack(id) ═══════ (sent per finished message)
```

Pixels are sent raw, with no compression or encoding. The bytes go out in the
source format (1 to 4 bytes per pixel), and the client expands them to RGBA.
Shapes are `[height, width]` on the server and `[width, height]` (egui's order)
on the wire and on the client.

### 1.2 The three image operations (`protocol/image.rs`)

| Header | Meaning | Payload |
|---|---|---|
| `Set` | whole new image | all pixels, chunked |
| `Update([x,y,w,h])` | a rectangle inside the existing image | the rectangle's pixels, as row strips |
| `Fill([w,h], rgba)` | solid colour | **none**, just the colour |

A large `Set` is split into **512 KiB chunks**:

```
 Set image of 3 MB:
 ┌───────┐┌───────┐┌───────┐┌───────┐┌───────┐┌─────┐
 │ Start ││ Batch ││ Batch ││ Batch ││ Batch ││ End │   ← only End is acked
 └───────┘└───────┘└───────┘└───────┘└───────┘└─────┘     and triggers a repaint
 small image → a single  [ All ]  message
```

The client collects the chunks in a `PartialImage` buffer. It replaces the
texture only when `End` arrives, so a half-loaded image is never shown. An
`Update` is split at row boundaries, and every strip is acknowledged.

### 1.3 Backpressure: one image in flight at a time

Each `Image` holds an `event` that is set while the server may send, plus a
one-slot `Buffer`:

```
 producer                      server Image                       client
    │ set_image(A)                  │                                │
    │──────────────────────────────►│ event set → send A now ───────►│
    │ set_image(B)                  │ A still in flight → B buffered │
    │ set_image(C)                  │ a set is pending → producer    │
    │       (blocks)                │   waits until it can send C    │
    │                               │◄────────────── Ack(A) ─────────│
    │                               │ ack drains B → send ──────────►│
    │                               │◄────────────── Ack(B) ─────────│
    │◄── C sent ────────────────────│ waiting producer sends C ─────►│
```

- **Set beats update:** a new `Set` discards any updates still buffered.
- **`force` update:** updating the same rectangle again with `force` replaces
  the buffered version instead of queueing behind it.
- **The result:** a slow client slows the server down to its pace, and memory
  does not grow without limit.

### 1.4 Connecting and reconnecting

```
 client connects ──► sync(): send the full current image (RGBA, Set) ──► wait for Ack
                     changes made before that Ack are folded into the local copy
                     instead of being queued separately

 client disconnects ─► reset(): generation++ , buffer cleared, waiting producers released
                       (any work stamped with the old generation is dropped)
```

`ImageMulti` (keyed images) works the same way, with three differences:

- All keys share **one ordered stream**.
- A reconnect first sends `Reset`, then each key in turn.
- `Remove` and `Reset` never jump ahead of chunks that were already queued.

### 1.5 Heartbeat

Because the server waits for acks, a client that vanishes silently would block
it forever. To prevent that, the server pings the client. If it receives no
frame before the heartbeat timeout, it treats the client as disconnected and
runs `reset()`. The timeout has to be longer than the largest image transfer
takes on the slowest expected network.

### 1.6 Key files

- `crates/egui-states/src/protocol/image.rs` — wire format
- `crates/egui-states/src/server_core/states/image/single.rs` — ack and buffer logic
- `crates/egui-states/src/server_core/states/image/multi.rs` — the keyed variant
- `crates/egui-states/src/server_core/transfer.rs` — `chunk_transfer`, `chunk_rows`
- `crates/egui-states/src/server_core/net/session.rs` (`DataReceiver`) — frame batching
- `crates/egui-states/src/client/states/image.rs` — client receive
- `docs/architecture.md` — the same model in prose

---

## 2. Inconsistency between `set` and `update`

> **Question:** `Set` sends the whole image (a large image is split into
> chunks) and the texture is updated after the full image is available on the
> client. But `update` splits a large rectangle into smaller rectangles (full
> lines) and the texture is updated for each transaction. Am I right?

Yes, with one qualifier: the split only happens for an update larger than
**10 MB**, because the two paths also use different chunk limits. `ImageMulti`
behaves the same way.

| | `Set` | `Update` |
|---|---|---|
| Split unit | byte chunks (`Start`/`Batch`/`End`) | independent row strips, each a complete `Update` message |
| Chunk limit | 512 KiB (`TRANSFER_CHUNK_SIZE`) | 10 MB (`MSG_SIZE_THRESHOLD`) |
| Client applies | once, at `End` (buffered in `PartialImage`) | every strip immediately (`set_partial`) |
| Ack | one, at `End` | one per strip |
| Server sends | all chunks queued at once | next strip only after the previous ack |
| Repaint request | at `End` | only on the last strip |

```
 Set (3 MB):      [Start][Batch][Batch][Batch][Batch][End]──► texture swapped once
                                                       ▲ ack
 Update (25 MB):  [strip 1]──►set_partial   [strip 2]──►set_partial   [strip 3]──►set_partial + repaint
                        ▲ ack                     ▲ ack                     ▲ ack
```

What this means in practice:

- **A large update is not atomic.** The texture is modified strip by strip.
  Only the last strip requests a repaint, but any repaint for another reason
  (mouse movement, an animation, another state changing) shows the
  half-updated rectangle.
- **Updates under 10 MB are atomic.** They go out as a single message, so the
  inconsistency only shows for big rectangles, e.g. more than about 2.5 MP of
  RGBA.
- **A large update is slower.** Each strip waits for the previous ack, so there
  is one round trip per strip. A `Set` streams all its chunks at once.
- **An update can go out as one frame of up to 10 MB.** That is exactly what
  the 512 KiB chunking was introduced to avoid, because of the quadratic cost
  of partial socket writes.
- **A `force` update can mix two versions.** It replaces the strips still
  buffered while earlier strips are already applied, so the client can end up
  with the top of the old update and the bottom of the new one. That is
  harmless if the next update covers the same rectangle, but it is another
  effect of the non-atomic path.

Relevant code: `chunk_rows` in `server_core/transfer.rs`, the limits in
`pack_set_data` / `pack_update_data` in `server_core/states/image/single.rs`,
and `update_image` in `client/states/image.rs`.

---

## 3. Suggested API improvement

> **Background (from the author):** The current `update` behaviour exists for
> the ExpertPI project (<https://github.com/tescan-orsay-holding/ExpertPI>).
> With very large images (more than 1k pixels per side), a histogram change
> had to update the whole image, and every change of the histogram was slow.
> Now the image is updated in strips from the top, and another histogram
> change forces a restart, which gives nicer behaviour.
>
> **Question:** Any suggestion how to improve the current egui-states API but
> still cover this use case?

**Recommendation:** keep the progressive strip behaviour, but make it an
explicit option of `update` instead of a side effect of the 10 MB limit, and
make the default atomic like `set`.

The use case needs two separate things, and today both are implicit:

| Property | What ExpertPI needs | How it happens today |
|---|---|---|
| **Progressive display** | Strips become visible as they arrive | Only when the rectangle is over 10 MB, in strips of about 10 MB; only the last strip requests a repaint |
| **Latest wins** | A new histogram change abandons the unfinished old one | `force=True`, and only if the rectangle is identical; otherwise the call blocks |

So a 2k×2k RGBA image (16 MB) gets just two strips, and anything under 10 MB
gets no progressive behaviour at all. The intermediate strips are probably
visible only because dragging the histogram repaints the UI anyway.

### 3.1 Proposed API

```python
image.update(arr, origin, update=True)                      # atomic, like set (default)
image.update(arr, origin, update=True, progressive=True)    # strips shown as they arrive
image.update(arr, origin, update=True, progressive=True, replace=True)   # ExpertPI histogram
```

- **Atomic (default):** the same `Start`/`Batch`/`End` framing and 512 KiB
  chunks as `set`. The client buffers the rectangle and calls `set_partial`
  once, with one ack. This removes the inconsistency from section 2.
- **`progressive=True`:** row strips at the same 512 KiB size, each applied and
  acknowledged on its own. Every strip requests a repaint when `update` is set,
  so the feedback no longer depends on incidental repaints.
- **`replace=True` (renamed `force`):** drops whatever part of the pending
  update has not been sent, and never blocks. The "same rectangle" rule could
  be relaxed to "the new rectangle covers the pending one".
- **`set` stays atomic.** It can change the image size, so a half-applied `set`
  has no sensible meaning. A `replace` option could be added there later for
  live-view streams.

The per-strip ack stays for progressive mode because it is what makes
cancellation possible: the server only ever has one strip in flight. The cost
is one round trip per strip, which is small for a local connection.

### 3.2 Alternative for the histogram case

Send the raw data once (for example a `Data` buffer of `u16`) and the histogram
levels as a small `Value`, and let the GUI apply the lookup table when it
builds the texture. A histogram change then sends a few bytes instead of the
whole image. This is already possible with the current API, but it moves the
mapping into the Rust GUI and only helps for lookup-table style changes, so it
complements the `update` change rather than replacing it.

### 3.3 Open points

- The `update` change alters the wire format, so `PROTOCOL_VERSION` needs the
  check described in `AGENT.md` (released value plus one, if not already
  bumped since the last release).
- Whether `force` is renamed to `replace` or kept as an alias (a rename is a
  breaking API change for ExpertPI).
- Whether the progressive strip size should be fixed at 512 KiB or
  configurable.
- The same changes apply to `ImageMulti` / `SingleImage.update`.
