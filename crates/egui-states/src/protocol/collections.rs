use serde::{Deserialize, Serialize};

#[cfg(any(feature = "server", feature = "python"))]
use crate::protocol::{FastVec, ServerHeader, serialize};

// A whole-collection replacement larger than `TRANSFER_CHUNK_SIZE` is split at
// item boundaries into `Begin`, any number of `Chunk`, and `End`; each carries the
// item count of its own message. The client applies the collection on `End`.

#[derive(Serialize, Deserialize)]
pub(crate) enum MapHeader {
    All(u64),
    Set,
    Remove,
    Begin(u64),
    Chunk(u64),
    End(u64),
}

#[derive(Serialize, Deserialize)]
pub(crate) enum VecHeader {
    All(u64),
    Set(u64),
    Add,
    Remove(u64),
    Begin(u64),
    Chunk(u64),
    End(u64),
}

/// Position of one message in a whole-collection replacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    All,
    Begin,
    Chunk,
    End,
}

#[cfg(any(feature = "server", feature = "python"))]
impl VecHeader {
    pub(crate) fn part(part: Part, count: u64) -> Self {
        match part {
            Part::All => Self::All(count),
            Part::Begin => Self::Begin(count),
            Part::Chunk => Self::Chunk(count),
            Part::End => Self::End(count),
        }
    }
}

#[cfg(any(feature = "server", feature = "python"))]
impl MapHeader {
    pub(crate) fn part(part: Part, count: u64) -> Self {
        match part {
            Part::All => Self::All(count),
            Part::Begin => Self::Begin(count),
            Part::Chunk => Self::Chunk(count),
            Part::End => Self::End(count),
        }
    }
}

/// Splits items of the given serialized sizes into messages of at most
/// `TRANSFER_CHUNK_SIZE` bytes, returning `(part, item count, bytes)` per
/// message.
///
/// An item larger than the chunk size travels alone. Items are limited to
/// `VALUE_MAX_SIZE`, so every message fits in one websocket frame. There is
/// always at least one message.
#[cfg(any(feature = "server", feature = "python"))]
pub(crate) fn split_collection(sizes: impl IntoIterator<Item = usize>) -> Vec<(Part, u64, usize)> {
    let mut chunks = vec![(Part::All, 0u64, 0usize)];
    for size in sizes {
        let (_, count, bytes) = chunks.last_mut().expect("at least one chunk");
        if *count > 0 && *bytes + size > crate::protocol::TRANSFER_CHUNK_SIZE {
            chunks.push((Part::Chunk, 1, size));
        } else {
            *count += 1;
            *bytes += size;
        }
    }

    if chunks.len() > 1 {
        chunks[0].0 = Part::Begin;
        chunks.last_mut().expect("at least one chunk").0 = Part::End;
    }
    chunks
}

/// Serializes a whole-collection replacement into messages of at most one
/// transfer chunk each, split at item boundaries.
///
/// `header` builds the message header from the part, its item count, its
/// payload size and whether it requests a UI update, which only the last
/// message does; `write` appends one item.
#[cfg(any(feature = "server", feature = "python"))]
pub(crate) fn serialize_replacement<I: Iterator + Clone>(
    items: I,
    size: impl Fn(&I::Item) -> usize,
    mut write: impl FnMut(&mut FastVec<32>, I::Item),
    header: impl Fn(Part, u64, u32, bool) -> ServerHeader,
    update: bool,
) -> Result<Vec<FastVec<32>>, &'static str> {
    let chunks = split_collection(items.clone().map(|item| size(&item)));
    let last = chunks.len() - 1;
    let mut items = items;
    let mut messages = Vec::with_capacity(chunks.len());
    for (i, (part, count, bytes)) in chunks.into_iter().enumerate() {
        let header = header(part, count, bytes as u32, update && i == last);
        let mut message = serialize(&header).map_err(|_| "Failed to serialize header")?;
        message.reserve_exact(bytes);
        items
            .by_ref()
            .take(count as usize)
            .for_each(|item| write(&mut message, item));
        messages.push(message);
    }
    Ok(messages)
}

/// A whole-collection replacement being assembled on the client.
///
/// The previous collection stays in place until the last part arrives. A
/// malformed or unexpected part discards the replacement, and a new `Begin`
/// replaces one that never finished.
#[cfg(feature = "client")]
pub(crate) struct Replacement<C>(parking_lot::Mutex<Option<C>>);

#[cfg(feature = "client")]
impl<C: Default> Replacement<C> {
    pub(crate) fn new() -> Self {
        Self(parking_lot::Mutex::new(None))
    }

    /// Applies one part and returns the complete collection after `All` or
    /// `End`. `decode` appends `count` items from `data`; `no_begin` gives the
    /// error for a part without a beginning.
    pub(crate) fn apply(
        &self,
        part: Part,
        count: u64,
        data: &[u8],
        decode: impl Fn(u64, &[u8], &mut C) -> Result<(), String>,
        no_begin: impl FnOnce() -> String,
    ) -> Result<Option<C>, String> {
        let mut pending = self.0.lock();
        match part {
            Part::All => {
                // Decode outside the lock; the caller swaps only on success,
                // so a malformed payload keeps the previous collection.
                *pending = None;
                drop(pending);
                let mut collection = C::default();
                decode(count, data, &mut collection)?;
                return Ok(Some(collection));
            }
            Part::Begin => *pending = Some(C::default()),
            Part::Chunk | Part::End => {}
        }
        let Some(collection) = pending.as_mut() else {
            return Err(no_begin());
        };
        if let Err(error) = decode(count, data, collection) {
            *pending = None;
            return Err(error);
        }
        Ok(if part == Part::End {
            pending.take()
        } else {
            None
        })
    }

    /// Drops a replacement cut short by a closed connection.
    pub(crate) fn clear(&self) {
        *self.0.lock() = None;
    }

    #[cfg(test)]
    pub(crate) fn is_pending(&self) -> bool {
        self.0.lock().is_some()
    }
}

#[cfg(all(test, any(feature = "server", feature = "python")))]
mod tests {
    use super::*;
    use crate::protocol::{TRANSFER_CHUNK_SIZE, VALUE_MAX_SIZE};

    #[test]
    fn a_collection_within_the_chunk_size_is_one_message() {
        assert_eq!(split_collection([]), vec![(Part::All, 0, 0)]);
        assert_eq!(
            split_collection([TRANSFER_CHUNK_SIZE - 1, 1]),
            vec![(Part::All, 2, TRANSFER_CHUNK_SIZE)]
        );
    }

    #[test]
    fn a_larger_collection_is_split_at_item_boundaries() {
        assert_eq!(
            split_collection([TRANSFER_CHUNK_SIZE, 1]),
            vec![(Part::Begin, 1, TRANSFER_CHUNK_SIZE), (Part::End, 1, 1)]
        );

        let item = TRANSFER_CHUNK_SIZE / 10;
        let chunks = split_collection([item; 25]);
        let parts: Vec<Part> = chunks.iter().map(|(part, ..)| *part).collect();
        assert_eq!(parts, [Part::Begin, Part::Chunk, Part::End]);
        assert_eq!(chunks.iter().map(|(_, count, _)| count).sum::<u64>(), 25);
        assert!(
            chunks
                .iter()
                .all(|(.., bytes)| *bytes <= TRANSFER_CHUNK_SIZE)
        );
    }

    #[test]
    fn an_item_larger_than_the_chunk_size_travels_alone() {
        let chunks = split_collection([1, VALUE_MAX_SIZE, 1]);
        assert_eq!(
            chunks,
            vec![
                (Part::Begin, 1, 1),
                (Part::Chunk, 1, VALUE_MAX_SIZE),
                (Part::End, 1, 1),
            ]
        );
    }
}
