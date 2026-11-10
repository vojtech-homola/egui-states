//! Splitting a transfer into messages.
//!
//! The packers of the Data, take and image states describe only their own
//! headers and payload copies; these functions decide where the chunk
//! boundaries fall and which messages may be merged with following ones.

use std::collections::VecDeque;
use std::ops::Range;

use crate::protocol::FastVec;

/// Messages of one transfer, each with whether it must go out in a frame of
/// its own (`true`) or may be merged with the messages queued after it.
pub(crate) type Messages = Vec<(FastVec<32>, bool)>;

/// Position of one message in a chunked transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Chunk {
    /// The whole transfer in one message.
    All,
    /// The first of several messages.
    Start,
    /// A message between the first and the last.
    Batch,
    /// The last of several messages; the client publishes the transfer here.
    End,
}

/// Splits a transfer of `total` bytes into messages of at most `chunk_limit`
/// payload bytes, cut at multiples of `unit` bytes.
///
/// `write` builds the message for one chunk from its position and its byte
/// range. A transfer that fits is one `All` message; otherwise it is a
/// `Start`, any number of `Batch` and an `End`. Only the `End` may be merged
/// with the messages queued after it.
pub(crate) fn chunk_transfer(
    total: usize,
    unit: usize,
    chunk_limit: usize,
    mut write: impl FnMut(Chunk, Range<usize>) -> Result<FastVec<32>, String>,
) -> Result<Messages, String> {
    if total <= chunk_limit {
        return Ok(vec![(write(Chunk::All, 0..total)?, true)]);
    }

    let chunk = (chunk_limit / unit * unit).max(unit);
    let mut messages = Vec::with_capacity(total.div_ceil(chunk));
    messages.push((write(Chunk::Start, 0..chunk)?, true));
    let mut start = chunk;
    while total - start > chunk {
        messages.push((write(Chunk::Batch, start..start + chunk)?, true));
        start += chunk;
    }
    messages.push((write(Chunk::End, start..total)?, false));
    Ok(messages)
}

/// Splits an image update of `rows` rows of `row_bytes` bytes into strips of
/// at most `chunk_limit` bytes (at least one row each).
///
/// Every strip is applied and acknowledged on its own. `write` builds the
/// message for the strip starting at its first row, with its row count and
/// whether the strip requests a repaint, which only the last one does when
/// `update` is set.
///
/// A strip is queued to go out in a frame of its own unless it is the last.
/// An update that fits in one strip is queued with `send_now = update`; this
/// quirk of the original packers is kept.
pub(crate) fn chunk_rows(
    rows: usize,
    row_bytes: usize,
    chunk_limit: usize,
    update: bool,
    mut write: impl FnMut(usize, usize, bool) -> Result<FastVec<32>, String>,
) -> Result<VecDeque<(FastVec<32>, bool)>, String> {
    if rows * row_bytes <= chunk_limit {
        return Ok(VecDeque::from([(write(0, rows, update)?, update)]));
    }

    let strip = (chunk_limit / row_bytes).max(1);
    let mut messages = VecDeque::with_capacity(rows.div_ceil(strip));
    let mut first = 0;
    while first < rows {
        let count = (rows - first).min(strip);
        let is_last = first + count == rows;
        messages.push_back((write(first, count, is_last && update)?, !is_last));
        first += count;
    }
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(total: usize, unit: usize, limit: usize) -> Vec<(Chunk, Range<usize>, bool)> {
        let mut chunks = Vec::new();
        let messages = chunk_transfer(total, unit, limit, |chunk, range| {
            chunks.push((chunk, range));
            Ok(FastVec::new())
        })
        .unwrap();
        chunks
            .into_iter()
            .zip(messages)
            .map(|((chunk, range), (_, send_now))| (chunk, range, send_now))
            .collect()
    }

    #[test]
    fn a_transfer_that_fits_is_one_message() {
        assert_eq!(layout(0, 2, 8), [(Chunk::All, 0..0, true)]);
        assert_eq!(layout(8, 2, 8), [(Chunk::All, 0..8, true)]);
    }

    #[test]
    fn chunks_are_whole_units_and_only_the_end_may_merge() {
        // 12-byte limit with 8-byte units leaves 8-byte chunks.
        assert_eq!(
            layout(20, 8, 12),
            [
                (Chunk::Start, 0..8, true),
                (Chunk::Batch, 8..16, true),
                (Chunk::End, 16..20, false),
            ]
        );
        assert_eq!(
            layout(9, 1, 8),
            [(Chunk::Start, 0..8, true), (Chunk::End, 8..9, false)]
        );
    }

    #[test]
    fn row_strips_request_a_repaint_only_at_the_end() {
        let mut strips = Vec::new();
        let messages = chunk_rows(5, 4, 8, true, |first, rows, repaint| {
            strips.push((first, rows, repaint));
            Ok(FastVec::new())
        })
        .unwrap();
        assert_eq!(strips, [(0, 2, false), (2, 2, false), (4, 1, true)]);
        let send_now: Vec<bool> = messages.iter().map(|(_, now)| *now).collect();
        assert_eq!(send_now, [true, true, false]);

        let mut single = Vec::new();
        let messages = chunk_rows(2, 4, 8, false, |first, rows, repaint| {
            single.push((first, rows, repaint));
            Ok(FastVec::new())
        })
        .unwrap();
        assert_eq!(single, [(0, 2, false)]);
        assert!(!messages[0].1);
    }
}
