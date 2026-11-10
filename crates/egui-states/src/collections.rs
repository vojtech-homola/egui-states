use serde::{Deserialize, Serialize};

// A whole-collection replacement larger than `MSG_SIZE_THRESHOLD` is split at
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
#[cfg(any(feature = "server", feature = "python"))]
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
        if *count > 0 && *bytes + size > crate::serialization::TRANSFER_CHUNK_SIZE {
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

#[cfg(all(test, any(feature = "server", feature = "python")))]
mod tests {
    use super::*;
    use crate::serialization::{TRANSFER_CHUNK_SIZE, VALUE_MAX_SIZE};

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
