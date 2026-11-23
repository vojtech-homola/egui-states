//! The headers of the messages each peer sends. Their variant order is the
//! wire format.

use serde::{Deserialize, Serialize};

use crate::protocol::collections::{MapHeader, VecHeader};
use crate::protocol::data::{DataHeader, DataMultiTakeHeader, DataTakeHeader, MultiDataHeader};
use crate::protocol::file_transfer::TransferFrame;
use crate::protocol::image::{ImageHeader, ImageMultiHeader};

// ServerHeader ---------------------------------------------------------------
// ----------------------------------------------------------------------------
#[derive(Serialize, Deserialize)]
pub(crate) enum ServerHeader {
    Value(u64, u32, u32),
    /// Id, type id, blocking, payload length.
    ValueTake(u64, u32, bool, u32),
    Static(u64, u32, u32),
    Image(u64, ImageHeader, u32),
    Data(u64, DataHeader),
    DataTake(u64, DataTakeHeader, bool),
    MultiData(u64, MultiDataHeader),
    DataMultiTake(u64, DataMultiTakeHeader),
    ValueVec(u64, u32, VecHeader, u32),
    ValueMap(u64, u32, MapHeader, u32),
    Update(f32),
    ImageMulti(u64, ImageMultiHeader),
    /// A file transfer frame: endpoint id, frame, payload length.
    Transfer(u64, TransferFrame, u32),
    /// The window and the chunk size of file transfers on this connection, in
    /// bytes. The first message of every connection.
    TransferLimits(u32, u32),
    /// Follows every frame of the low-priority lane. The client answers with
    /// [`ClientHeader::LaneAck`] once it has handled everything before it,
    /// which returns that frame's bytes to the lane's window.
    LaneMark,
    /// The server asks its connect-request callback whether to accept the
    /// client and answers within this many milliseconds. Sent before any
    /// other message; it does not mean the client is accepted.
    ConnectPending(u32),
    /// The server's heartbeat timeout in milliseconds: a client that receives
    /// nothing for this long treats the connection as lost. Follows
    /// [`ServerHeader::TransferLimits`] when the heartbeat is on; without it
    /// the client waits for the server indefinitely.
    Heartbeat(u32),
    /// Sent when the server had nothing else to send for a heartbeat
    /// interval, so that the client keeps hearing from it.
    KeepAlive,
}

#[derive(Serialize, Deserialize)]
pub(crate) enum ClientHeader {
    Value(u64, u32, bool, u32),
    Signal(u64, u32, u32),
    Ack(u64),
    Message(u32),
    Handshake(u16, Option<u64>, Option<String>),
    /// A file transfer frame: endpoint id, frame, payload length.
    Transfer(u64, TransferFrame, u32),
    /// Answers the oldest unanswered [`ServerHeader::LaneMark`].
    LaneAck,
    /// Acknowledges a transfer of one key of a keyed state: state id, key.
    AckKey(u64, u32),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::data::HeaderUpdate;
    use crate::protocol::fastvec::{FastVec, contents, serialize};

    fn first_byte<const N: usize>(data: &FastVec<N>) -> u8 {
        match data {
            FastVec::Heap(data) => data[0],
            FastVec::Stack(data) => data.as_ref()[0],
        }
    }

    #[test]
    fn image_multi_is_appended_to_the_server_header_protocol() {
        let image: FastVec<32> = serialize(&ServerHeader::Image(
            1,
            ImageHeader::Fill([1, 1], [0; 4], HeaderUpdate::Keep),
            0,
        ))
        .unwrap();
        let update: FastVec<32> = serialize(&ServerHeader::Update(0.0)).unwrap();
        let image_multi: FastVec<32> =
            serialize(&ServerHeader::ImageMulti(1, ImageMultiHeader::Reset)).unwrap();

        assert_eq!(first_byte(&image), 3);
        assert_eq!(first_byte(&update), 10);
        assert_eq!(first_byte(&image_multi), 11);
    }

    /// Variant indexes are the wire tags of both headers; reordering or inserting
    /// a variant breaks peers built from another version.
    #[test]
    fn header_variant_indexes_are_stable() {
        use crate::protocol::collections::{MapHeader, VecHeader};
        use crate::protocol::data::{
            DataHeader, DataMultiTakeHeader, DataTakeHeader, DataType, HeaderUpdate,
            MultiDataHeader,
        };

        let server = [
            ServerHeader::Value(1, 2, 0),
            ServerHeader::ValueTake(1, 2, false, 0),
            ServerHeader::Static(1, 2, 0),
            ServerHeader::Image(1, ImageHeader::Fill([1, 1], [0; 4], HeaderUpdate::Keep), 0),
            ServerHeader::Data(1, DataHeader::Clear(HeaderUpdate::Keep)),
            ServerHeader::DataTake(1, DataTakeHeader::Batch(0), false),
            ServerHeader::MultiData(1, MultiDataHeader::Reset),
            ServerHeader::DataMultiTake(1, DataMultiTakeHeader::Reset),
            ServerHeader::ValueVec(1, 2, VecHeader::Add, 0),
            ServerHeader::ValueMap(1, 2, MapHeader::Set, 0),
            ServerHeader::Update(0.0),
            ServerHeader::ImageMulti(1, ImageMultiHeader::Reset),
            ServerHeader::Transfer(1, TransferFrame::Reject { serial: 0 }, 0),
            ServerHeader::TransferLimits(1, 1),
            ServerHeader::LaneMark,
            ServerHeader::ConnectPending(1),
            ServerHeader::Heartbeat(1),
            ServerHeader::KeepAlive,
        ];
        for (index, header) in server.iter().enumerate() {
            let data: FastVec<32> = serialize(header).unwrap();
            assert_eq!(
                first_byte(&data),
                index as u8,
                "ServerHeader variant {index}"
            );
        }

        let client = [
            ClientHeader::Value(1, 2, false, 0),
            ClientHeader::Signal(1, 2, 0),
            ClientHeader::Ack(1),
            ClientHeader::Message(0),
            ClientHeader::Handshake(1, None, None),
            ClientHeader::Transfer(1, TransferFrame::Reject { serial: 0 }, 0),
            ClientHeader::LaneAck,
            ClientHeader::AckKey(1, 2),
        ];
        for (index, header) in client.iter().enumerate() {
            let data: FastVec<32> = serialize(header).unwrap();
            assert_eq!(
                first_byte(&data),
                index as u8,
                "ClientHeader variant {index}"
            );
        }

        // Nested tags travel inside the headers above.
        let data_types = [
            DataType::U8,
            DataType::U16,
            DataType::U32,
            DataType::U64,
            DataType::I8,
            DataType::I16,
            DataType::I32,
            DataType::I64,
            DataType::F32,
            DataType::F64,
        ];
        for (index, data_type) in data_types.iter().enumerate() {
            let data: FastVec<32> = serialize(data_type).unwrap();
            assert_eq!(first_byte(&data), index as u8, "DataType variant {index}");
        }
    }

    fn assert_tags<T: Serialize>(name: &str, variants: &[T]) {
        for (index, variant) in variants.iter().enumerate() {
            let data: FastVec<32> = serialize(variant).unwrap();
            assert_eq!(first_byte(&data), index as u8, "{name} variant {index}");
        }
    }

    /// The headers nested inside `ServerHeader` are wire tags too.
    #[test]
    fn nested_header_variant_indexes_are_stable() {
        use crate::protocol::collections::{MapHeader, VecHeader};
        use crate::protocol::data::{
            DataHeader, DataMultiTakeHeader, DataTakeHeader, DataType, HeaderBytes, HeaderUpdate,
            MultiDataHeader, TransportType,
        };
        use crate::protocol::image::{ImageSetHeader, ImageType, ImageUpdateHeader};

        let header = HeaderBytes {
            type_id: 0,
            size: 0,
        };
        assert_tags(
            "HeaderUpdate",
            &[HeaderUpdate::Keep, HeaderUpdate::Set(header)],
        );
        assert_tags(
            "TransportType",
            &[
                TransportType::Set(0),
                TransportType::Add(0),
                TransportType::Replace(0, 0),
            ],
        );
        assert_tags(
            "DataHeader",
            &[
                DataHeader::All(DataType::U8, TransportType::Set(0), 0, HeaderUpdate::Keep),
                DataHeader::StartBatch(0, 0),
                DataHeader::Batch(0),
                DataHeader::End(DataType::U8, TransportType::Set(0), 0, HeaderUpdate::Keep),
                DataHeader::Drain(0, 0, HeaderUpdate::Keep),
                DataHeader::Clear(HeaderUpdate::Keep),
                DataHeader::Header(header),
                DataHeader::Reset,
            ],
        );
        assert_tags(
            "DataTakeHeader",
            &[
                DataTakeHeader::All(DataType::U8, 0, 0, None),
                DataTakeHeader::StartBatch(0, 0),
                DataTakeHeader::Batch(0),
                DataTakeHeader::End(DataType::U8, 0, 0, None),
            ],
        );
        assert_tags(
            "MultiDataHeader",
            &[
                MultiDataHeader::Remove(0),
                MultiDataHeader::Modify(0, DataHeader::Clear(HeaderUpdate::Keep)),
                MultiDataHeader::Reset,
            ],
        );
        assert_tags(
            "DataMultiTakeHeader",
            &[
                DataMultiTakeHeader::Remove(0),
                DataMultiTakeHeader::Modify(0, DataTakeHeader::Batch(0), false),
                DataMultiTakeHeader::Reset,
            ],
        );
        assert_tags(
            "ImageType",
            &[
                ImageType::Color,
                ImageType::ColorAlpha,
                ImageType::Gray,
                ImageType::GrayAlpha,
            ],
        );
        assert_tags(
            "ImageSetHeader",
            &[
                ImageSetHeader::All([0; 2], HeaderUpdate::Keep),
                ImageSetHeader::Start([0; 2], 0),
                ImageSetHeader::Batch(0),
                ImageSetHeader::End(0, HeaderUpdate::Keep),
            ],
        );
        assert_tags(
            "ImageUpdateHeader",
            &[
                ImageUpdateHeader::All([0; 4], HeaderUpdate::Keep),
                ImageUpdateHeader::Start([0; 4], 0),
                ImageUpdateHeader::Batch(0),
                ImageUpdateHeader::End(0, HeaderUpdate::Keep),
            ],
        );
        assert_tags(
            "ImageHeader",
            &[
                ImageHeader::Set(ImageSetHeader::Batch(0), ImageType::Color),
                ImageHeader::Update(ImageUpdateHeader::Batch(0), ImageType::Color),
                ImageHeader::Fill([0; 2], [0; 4], HeaderUpdate::Keep),
                ImageHeader::Header(HeaderBytes {
                    type_id: 0,
                    size: 0,
                }),
            ],
        );
        assert_tags(
            "ImageMultiHeader",
            &[
                ImageMultiHeader::Remove(0),
                ImageMultiHeader::Modify(
                    0,
                    ImageHeader::Fill([0; 2], [0; 4], HeaderUpdate::Keep),
                    0,
                ),
                ImageMultiHeader::Reset,
            ],
        );
        assert_tags(
            "VecHeader",
            &[
                VecHeader::All(0),
                VecHeader::Set(0),
                VecHeader::Add,
                VecHeader::Remove(0),
                VecHeader::Begin(0),
                VecHeader::Chunk(0),
                VecHeader::End(0),
            ],
        );
        assert_tags(
            "MapHeader",
            &[
                MapHeader::All(0),
                MapHeader::Set,
                MapHeader::Remove,
                MapHeader::Begin(0),
                MapHeader::Chunk(0),
                MapHeader::End(0),
            ],
        );
    }

    /// Exact encodings of representative headers; any change here is a wire
    /// format change.
    #[test]
    fn representative_header_bytes_are_stable() {
        use crate::protocol::collections::VecHeader;
        use crate::protocol::data::{
            DataHeader, DataMultiTakeHeader, DataTakeHeader, DataType, HeaderBytes, HeaderUpdate,
            MultiDataHeader, TransportType,
        };
        use crate::protocol::image::{ImageSetHeader, ImageType, ImageUpdateHeader};

        fn bytes<T: Serialize>(value: &T) -> Vec<u8> {
            contents(&serialize::<T, 64>(value).unwrap())
        }

        // postcard: enum tags and integers are varints, `bool` is one byte,
        // `f32` is four little-endian bytes and arrays have no length prefix.
        assert_eq!(bytes(&ServerHeader::Value(5, 7, 3)), [0, 5, 7, 3]);
        assert_eq!(
            bytes(&ServerHeader::ValueTake(300, 1, true, 2)),
            [1, 0xac, 0x02, 1, 1, 2]
        );
        assert_eq!(
            bytes(&ServerHeader::Data(
                5,
                DataHeader::All(DataType::U16, TransportType::Set(3), 6, HeaderUpdate::Keep)
            )),
            [4, 5, 0, 1, 0, 3, 6, 0]
        );
        assert_eq!(
            bytes(&ServerHeader::Data(
                5,
                DataHeader::End(
                    DataType::F64,
                    TransportType::Replace(2, 4),
                    32,
                    HeaderUpdate::Keep
                )
            )),
            [4, 5, 3, 9, 2, 2, 4, 32, 0]
        );
        assert_eq!(
            bytes(&ServerHeader::Data(
                5,
                DataHeader::Drain(1, 2, HeaderUpdate::Keep)
            )),
            [4, 5, 4, 1, 2, 0]
        );
        assert_eq!(
            bytes(&ServerHeader::DataTake(
                5,
                DataTakeHeader::All(DataType::U8, 4, 4, None),
                true
            )),
            [5, 5, 0, 0, 4, 4, 0, 1]
        );
        assert_eq!(
            bytes(&ServerHeader::MultiData(
                5,
                MultiDataHeader::Modify(9, DataHeader::Clear(HeaderUpdate::Keep))
            )),
            [6, 5, 1, 9, 5, 0]
        );
        assert_eq!(
            bytes(&ServerHeader::DataMultiTake(
                5,
                DataMultiTakeHeader::Modify(9, DataTakeHeader::StartBatch(10, 4), false)
            )),
            [7, 5, 1, 9, 1, 10, 4, 0]
        );
        let header = HeaderBytes {
            type_id: 300,
            size: 2,
        };
        assert_eq!(
            bytes(&ServerHeader::Data(5, DataHeader::Header(header))),
            [4, 5, 6, 0xac, 0x02, 2]
        );
        assert_eq!(
            bytes(&ServerHeader::Data(
                5,
                DataHeader::Clear(HeaderUpdate::Set(header))
            )),
            [4, 5, 5, 1, 0xac, 0x02, 2]
        );
        assert_eq!(bytes(&ServerHeader::Data(5, DataHeader::Reset)), [4, 5, 7]);
        assert_eq!(
            bytes(&ServerHeader::DataTake(
                5,
                DataTakeHeader::End(DataType::U8, 4, 4, Some(header)),
                false
            )),
            [5, 5, 3, 0, 4, 4, 1, 0xac, 0x02, 2, 0]
        );
        assert_eq!(
            bytes(&ServerHeader::Image(
                5,
                ImageHeader::Set(
                    ImageSetHeader::All([2, 3], HeaderUpdate::Keep),
                    ImageType::Gray
                ),
                6
            )),
            [3, 5, 0, 0, 2, 3, 0, 2, 6]
        );
        assert_eq!(
            bytes(&ServerHeader::Image(
                5,
                ImageHeader::Fill([2, 1], [1, 2, 3, 4], HeaderUpdate::Set(header)),
                0
            )),
            [3, 5, 2, 2, 1, 1, 2, 3, 4, 1, 0xac, 0x02, 2, 0]
        );
        assert_eq!(
            bytes(&ServerHeader::Image(5, ImageHeader::Header(header), 0)),
            [3, 5, 3, 0xac, 0x02, 2, 0]
        );
        assert_eq!(
            bytes(&ServerHeader::ImageMulti(
                5,
                ImageMultiHeader::Modify(
                    1,
                    ImageHeader::Update(
                        ImageUpdateHeader::All([0, 1, 2, 3], HeaderUpdate::Keep),
                        ImageType::ColorAlpha
                    ),
                    24
                )
            )),
            [11, 5, 1, 1, 1, 0, 0, 1, 2, 3, 0, 1, 24]
        );
        assert_eq!(
            bytes(&ServerHeader::ValueVec(5, 7, VecHeader::Remove(2), 0)),
            [8, 5, 7, 3, 2, 0]
        );
        assert_eq!(bytes(&ServerHeader::Update(1.0)), [10, 0, 0, 0x80, 0x3f]);
        assert_eq!(bytes(&ClientHeader::Ack(300)), [2, 0xac, 0x02]);
        assert_eq!(bytes(&ClientHeader::AckKey(300, 5)), [7, 0xac, 0x02, 5]);
        assert_eq!(
            bytes(&ClientHeader::Handshake(7, Some(1), None)),
            [4, 7, 1, 1, 0]
        );
    }
}
