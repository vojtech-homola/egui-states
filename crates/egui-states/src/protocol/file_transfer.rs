//! Wire types of the file transfer endpoints (`Download` and `Upload`).
//!
//! Both peers use the same frames: each is the sender of a transfer in one
//! direction and the receiver in the other. A frame travels in a
//! [`ServerHeader::Transfer`] or [`ClientHeader::Transfer`] message together
//! with the endpoint id and the length of the payload that follows it.

use serde::{Deserialize, Serialize};

use crate::protocol::{ClientHeader, ServerHeader};

/// One frame of a transfer.
///
/// `serial` is allocated by the sender of the transfer, per endpoint. It tells
/// the frames of one transfer from late frames of an earlier one; it starts
/// again with every connection, so it is never a long-lived key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TransferFrame {
    /// Sender to receiver. Payload: the postcard-encoded [`OfferBody`],
    /// followed by the metadata bytes.
    Offer { serial: u32, meta_type: u32 },
    /// Sender to receiver, in the data lane. Payload: raw bytes of the
    /// concatenated files. `last` marks the final chunk of the transfer.
    Chunk { serial: u32, last: bool },
    /// Receiver to sender. Grants the initial window of credit.
    Accept { serial: u32, mode: AcceptMode },
    /// Receiver to sender. Payload: the reason as UTF-8.
    Reject { serial: u32 },
    /// Receiver to sender: `bytes` more may be sent.
    Credit { serial: u32, bytes: u32 },
    /// Either side, for a cancel by the user. Payload: the reason as UTF-8.
    Cancel { serial: u32 },
    /// Either side. Without `error`, the receiver confirms that everything is
    /// saved. With `error` (a [`ErrorCode`]), either side reports that the
    /// transfer failed; the payload is the message as UTF-8.
    Finished { serial: u32, error: Option<u8> },
    /// Server to client, browser downloads only: the ticket for one file
    /// (`file` is its index, `u32::MAX` for the bundle).
    Ticket {
        serial: u32,
        file: u32,
        ticket: [u8; 16],
    },
    /// Server to client, browser downloads only: what the server has sent.
    HttpProgress {
        serial: u32,
        done: u64,
        files_done: u32,
    },
}

impl TransferFrame {
    /// The transfer the frame belongs to.
    pub(crate) fn serial(&self) -> u32 {
        match *self {
            Self::Offer { serial, .. }
            | Self::Chunk { serial, .. }
            | Self::Accept { serial, .. }
            | Self::Reject { serial }
            | Self::Credit { serial, .. }
            | Self::Cancel { serial }
            | Self::Finished { serial, .. }
            | Self::Ticket { serial, .. }
            | Self::HttpProgress { serial, .. } => serial,
        }
    }
}

/// How the receiver takes the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AcceptMode {
    /// Chunks over the WebSocket.
    Lane,
    /// One browser download per file over HTTP.
    Http,
    /// One browser download of all files as an uncompressed zip.
    HttpBundled,
}

/// The files of an offer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OfferBody {
    /// The file name, or the name of the set.
    pub(crate) name: String,
    pub(crate) files: Vec<FileEntry>,
}

/// One file of an offer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FileEntry {
    /// A relative name with `/` between components.
    pub(crate) name: String,
    pub(crate) size: u64,
    /// Milliseconds since the Unix epoch.
    pub(crate) modified: Option<i64>,
}

/// Failure kinds carried by [`TransferFrame::Finished`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ErrorCode {
    Io = 1,
    Protocol = 2,
    TooLarge = 3,
    Unsupported = 4,
    Meta = 5,
}

impl ErrorCode {
    pub(crate) fn from_u8(code: u8) -> Self {
        match code {
            1 => Self::Io,
            3 => Self::TooLarge,
            4 => Self::Unsupported,
            5 => Self::Meta,
            _ => Self::Protocol,
        }
    }
}

/// Encodes one transfer message as the server sends it.
pub(crate) fn encode_server(endpoint: u64, frame: &TransferFrame, payload: &[u8]) -> Vec<u8> {
    encode(
        &ServerHeader::Transfer(endpoint, *frame, payload.len() as u32),
        payload,
    )
}

/// Encodes one transfer message as the client sends it.
pub(crate) fn encode_client(endpoint: u64, frame: &TransferFrame, payload: &[u8]) -> Vec<u8> {
    encode(
        &ClientHeader::Transfer(endpoint, *frame, payload.len() as u32),
        payload,
    )
}

fn encode<H: Serialize>(header: &H, payload: &[u8]) -> Vec<u8> {
    // The headers hold only integers, booleans and fixed arrays, so encoding
    // cannot fail.
    let mut data = postcard::to_stdvec(header).expect("transfer header encodes");
    data.extend_from_slice(payload);
    data
}

/// Encodes an offer's payload: the body, then the metadata.
pub(crate) fn encode_offer(body: &OfferBody, meta: &[u8]) -> Result<Vec<u8>, String> {
    let mut data = postcard::to_stdvec(body).map_err(|e| e.to_string())?;
    data.extend_from_slice(meta);
    Ok(data)
}

/// Splits an offer's payload into the body and the metadata bytes.
pub(crate) fn decode_offer(payload: &[u8]) -> Result<(OfferBody, &[u8]), String> {
    postcard::take_from_bytes::<OfferBody>(payload)
        .map_err(|e| format!("invalid transfer offer: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_tags_are_stable() {
        let frames = [
            TransferFrame::Offer {
                serial: 1,
                meta_type: 0,
            },
            TransferFrame::Chunk {
                serial: 1,
                last: false,
            },
            TransferFrame::Accept {
                serial: 1,
                mode: AcceptMode::Lane,
            },
            TransferFrame::Reject { serial: 1 },
            TransferFrame::Credit {
                serial: 1,
                bytes: 0,
            },
            TransferFrame::Cancel { serial: 1 },
            TransferFrame::Finished {
                serial: 1,
                error: None,
            },
            TransferFrame::Ticket {
                serial: 1,
                file: 0,
                ticket: [0; 16],
            },
            TransferFrame::HttpProgress {
                serial: 1,
                done: 0,
                files_done: 0,
            },
        ];
        for (index, frame) in frames.iter().enumerate() {
            let data = postcard::to_stdvec(frame).unwrap();
            assert_eq!(data[0], index as u8, "TransferFrame variant {index}");
        }

        let modes = [AcceptMode::Lane, AcceptMode::Http, AcceptMode::HttpBundled];
        for (index, mode) in modes.iter().enumerate() {
            let data = postcard::to_stdvec(mode).unwrap();
            assert_eq!(data[0], index as u8, "AcceptMode variant {index}");
        }
    }

    #[test]
    fn offer_payload_round_trips() {
        let body = OfferBody {
            name: "run42".to_string(),
            files: vec![
                FileEntry {
                    name: "run42/a.tiff".to_string(),
                    size: 10,
                    modified: Some(1_700_000_000_000),
                },
                FileEntry {
                    name: "run42/b.json".to_string(),
                    size: 0,
                    modified: None,
                },
            ],
        };
        let payload = encode_offer(&body, &[7, 8, 9]).unwrap();
        let (decoded, meta) = decode_offer(&payload).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(meta, &[7, 8, 9]);
    }

    #[test]
    fn worst_case_header_is_small() {
        let header = ServerHeader::Transfer(
            u64::MAX,
            TransferFrame::Ticket {
                serial: u32::MAX,
                file: u32::MAX,
                ticket: [0xff; 16],
            },
            u32::MAX,
        );
        let data = postcard::to_stdvec(&header).unwrap();
        assert!(data.len() <= 64, "{} bytes", data.len());
    }
}
