use postcard::ser_flavors::Flavor;
use serde::{Deserialize, Serialize};

use crate::collections::{MapHeader, VecHeader};
use crate::data_transport;
use crate::image_transport::{ImageHeader, ImageMultiHeader};

// TODO: make these constants configurable
pub(crate) const VALUE_MAX_SIZE: usize = 1024 * 1024; // 1 MB
pub(crate) const MSG_SIZE_THRESHOLD: usize = 1024 * 1024 * 10; // 10 MB
/// Chunk size of bulk transfers: Data, takes, image sets and collection
/// replacements. Much smaller than `MSG_SIZE_THRESHOLD`, the cap for merging
/// small messages into one frame, because the WebSocket library moves the
/// unsent rest of a message after every partial socket write, which costs
/// time quadratic in the message size.
#[cfg(any(feature = "server", feature = "python"))]
pub(crate) const TRANSFER_CHUNK_SIZE: usize = 512 * 1024;
#[cfg(any(
    all(feature = "client", not(target_arch = "wasm32")),
    feature = "server",
    feature = "python"
))]
pub(crate) const WEBSOCKET_MAX_SIZE: usize = MSG_SIZE_THRESHOLD + MSG_SIZE_THRESHOLD / 5; // 12 MB
pub(crate) const MAX_MSG_COUNT: usize = 10;

/// Check the size of a serialized value against the maximum allowed size.
///
/// Both sides of the connection use this, so the limit and the reported message
/// stay identical for values sent to the server and values sent to the client.
#[inline]
pub(crate) fn check_value_size(name: &str, size: usize) -> Result<(), String> {
    if size > VALUE_MAX_SIZE {
        return Err(format!(
            "value '{}' is too large: {} bytes, max {} bytes",
            name, size, VALUE_MAX_SIZE
        ));
    }
    Ok(())
}

pub(crate) struct StackVec<const N: usize>([u8; N], usize);

impl<const N: usize> AsRef<[u8]> for StackVec<N> {
    fn as_ref(&self) -> &[u8] {
        &self.0[..self.1]
    }
}

// FastVec ---------------------------------------------------------------
// -----------------------------------------------------------------------
pub(crate) enum FastVec<const N: usize> {
    Heap(Vec<u8>),
    Stack(StackVec<N>),
}

impl<const N: usize> FastVec<N> {
    #[inline]
    pub fn new() -> Self {
        Self::Stack(StackVec([0; N], 0))
    }

    #[cfg(any(feature = "server", feature = "python"))]
    #[inline]
    pub(crate) fn new_heap() -> Self {
        Self::Heap(Vec::new())
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[inline]
    pub fn into_bytes(self) -> bytes::Bytes {
        match self {
            Self::Heap(vec) => bytes::Bytes::from_owner(vec),
            Self::Stack(vec) => bytes::Bytes::from_owner(vec),
        }
    }

    #[cfg(target_arch = "wasm32")]
    #[inline]
    pub fn into_vec(self) -> Vec<u8> {
        match self {
            Self::Heap(vec) => vec,
            Self::Stack(stack_vec) => stack_vec.0[..stack_vec.1].to_vec(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Self::Heap(vec) => vec.len(),
            Self::Stack(stack_vec) => stack_vec.1,
        }
    }

    #[cfg(any(feature = "server", feature = "python"))]
    pub(crate) fn reserve_exact(&mut self, additional: usize) {
        match self {
            Self::Heap(vec) => vec.reserve_exact(additional),
            Self::Stack(stack_vec) => {
                if stack_vec.1 + additional > N {
                    let mut new_vec = Vec::with_capacity(stack_vec.1 + additional);
                    new_vec.extend_from_slice(&stack_vec.0[..stack_vec.1]);
                    *self = Self::Heap(new_vec);
                }
            }
        }
    }

    #[cfg(any(feature = "server", feature = "python"))]
    pub fn extend_from_slice(&mut self, data: &[u8]) {
        match self {
            Self::Heap(vec) => {
                vec.extend_from_slice(data);
            }
            Self::Stack(stack_vec) => {
                if stack_vec.1 + data.len() <= N {
                    stack_vec.0[stack_vec.1..stack_vec.1 + data.len()].copy_from_slice(data);
                    stack_vec.1 += data.len();
                } else {
                    let mut new_vec = Vec::with_capacity(stack_vec.1 + data.len());
                    new_vec.extend_from_slice(&stack_vec.0[..stack_vec.1]);
                    new_vec.extend_from_slice(data);
                    *self = Self::Heap(new_vec);
                }
            }
        }
    }

    pub fn extend_from_data<const M: usize>(&mut self, data: &FastVec<M>) {
        match self {
            Self::Heap(vec) => match data {
                FastVec::Heap(dvec) => vec.extend_from_slice(dvec),
                FastVec::Stack(dvec) => vec.extend_from_slice(dvec.as_ref()),
            },
            Self::Stack(stack_vec) => match data {
                FastVec::Heap(dvec) => {
                    if stack_vec.1 + dvec.len() <= N {
                        stack_vec.0[stack_vec.1..stack_vec.1 + dvec.len()].copy_from_slice(dvec);
                        stack_vec.1 += dvec.len();
                    } else {
                        let mut new_vec = Vec::with_capacity(stack_vec.1 + dvec.len());
                        new_vec.extend_from_slice(&stack_vec.0[..stack_vec.1]);
                        new_vec.extend_from_slice(dvec);
                        *self = Self::Heap(new_vec);
                    }
                }
                FastVec::Stack(dvec) => {
                    if stack_vec.1 + dvec.1 <= N {
                        stack_vec.0[stack_vec.1..stack_vec.1 + dvec.1]
                            .copy_from_slice(&dvec.0[..dvec.1]);
                        stack_vec.1 += dvec.1;
                    } else {
                        let mut new_vec = Vec::with_capacity(stack_vec.1 + dvec.1);
                        new_vec.extend_from_slice(&stack_vec.0[..stack_vec.1]);
                        new_vec.extend_from_slice(&dvec.0[..dvec.1]);
                        *self = Self::Heap(new_vec);
                    }
                }
            },
        }
    }
}

impl<const N: usize> Flavor for FastVec<N> {
    type Output = Self;

    fn try_push(&mut self, data: u8) -> postcard::Result<()> {
        match self {
            Self::Heap(vec) => {
                vec.push(data);
                Ok(())
            }
            Self::Stack(stack_vec) => {
                if stack_vec.1 < N {
                    stack_vec.0[stack_vec.1] = data;
                    stack_vec.1 += 1;
                    Ok(())
                } else {
                    let mut new_vec = Vec::with_capacity(stack_vec.1 + 1);
                    new_vec.extend_from_slice(&stack_vec.0);
                    new_vec.push(data);
                    *self = Self::Heap(new_vec);
                    Ok(())
                }
            }
        }
    }

    fn try_extend(&mut self, data: &[u8]) -> postcard::Result<()> {
        match self {
            Self::Heap(vec) => {
                vec.extend_from_slice(data);
                Ok(())
            }
            Self::Stack(stack_vec) => {
                if stack_vec.1 + data.len() <= N {
                    stack_vec.0[stack_vec.1..stack_vec.1 + data.len()].copy_from_slice(data);
                    stack_vec.1 += data.len();
                    Ok(())
                } else {
                    let mut new_vec = Vec::with_capacity(stack_vec.1 + data.len());
                    new_vec.extend_from_slice(&stack_vec.0[..stack_vec.1]);
                    new_vec.extend_from_slice(data);
                    *self = Self::Heap(new_vec);
                    Ok(())
                }
            }
        }
    }

    fn finalize(self) -> postcard::Result<Self::Output> {
        Ok(self)
    }
}

// ServerHeader ---------------------------------------------------------------
// ----------------------------------------------------------------------------
#[derive(Serialize, Deserialize)]
pub(crate) enum ServerHeader {
    Value(u64, u32, bool, u32),
    ValueTake(u64, u32, bool, bool, u32),
    Static(u64, u32, bool, u32),
    Image(u64, ImageHeader, u32),
    Data(u64, data_transport::DataHeader),
    DataTake(u64, data_transport::DataTakeHeader, bool),
    MultiData(u64, data_transport::MultiDataHeader),
    DataMultiTake(u64, data_transport::DataMultiTakeHeader),
    ValueVec(u64, u32, bool, VecHeader, u32),
    ValueMap(u64, u32, bool, MapHeader, u32),
    Update(f32),
    ImageMulti(u64, ImageMultiHeader),
}

#[cfg(any(feature = "server", feature = "python"))]
impl ServerHeader {
    pub fn serialize_value<const N: usize>(
        id: u64,
        type_id: u32,
        update: bool,
        value_data: &[u8],
    ) -> Result<FastVec<N>, &'static str> {
        let header = ServerHeader::Value(id, type_id, update, value_data.len() as u32);
        let mut data = FastVec::<N>::new();
        serialize_to_data(&header, &mut data).map_err(|_| "Failed to serialize header")?;
        data.extend_from_slice(value_data);
        Ok(data)
    }

    pub fn serialize_static<const N: usize>(
        id: u64,
        type_id: u32,
        update: bool,
        value_data: &[u8],
    ) -> Result<FastVec<N>, &'static str> {
        let header = ServerHeader::Static(id, type_id, update, value_data.len() as u32);
        let mut data = FastVec::<N>::new();
        serialize_to_data(&header, &mut data).map_err(|_| "Failed to serialize header")?;
        data.extend_from_slice(value_data);
        Ok(data)
    }

    pub fn serialize_value_take<const N: usize>(
        id: u64,
        type_id: u32,
        blocking: bool,
        update: bool,
        value_data: &[u8],
    ) -> Result<FastVec<N>, &'static str> {
        let header =
            ServerHeader::ValueTake(id, type_id, blocking, update, value_data.len() as u32);
        let mut data = FastVec::<N>::new();
        serialize_to_data(&header, &mut data).map_err(|_| "Failed to serialize header")?;
        data.extend_from_slice(value_data);
        Ok(data)
    }
}

#[cfg(feature = "client")]
impl ServerHeader {
    /// Bytes of payload that follow this header in the frame.
    pub(crate) fn payload_len(&self) -> u32 {
        use crate::data_transport::{DataMultiTakeHeader, MultiDataHeader};

        match self {
            Self::Value(.., size)
            | Self::ValueTake(.., size)
            | Self::Static(.., size)
            | Self::Image(_, _, size)
            | Self::ValueVec(.., size)
            | Self::ValueMap(.., size) => *size,
            Self::Data(_, header) => header.payload_len(),
            Self::DataTake(_, header, _) => header.payload_len(),
            Self::MultiData(_, MultiDataHeader::Modify(_, header)) => header.payload_len(),
            Self::DataMultiTake(_, DataMultiTakeHeader::Modify(_, header, _)) => {
                header.payload_len()
            }
            Self::ImageMulti(_, ImageMultiHeader::Modify(_, _, size)) => *size,
            Self::MultiData(..) | Self::DataMultiTake(..) | Self::ImageMulti(..) => 0,
            Self::Update(_) => 0,
        }
    }

    /// Whether the message asks the client to repaint once it is applied.
    pub(crate) fn update_flag(&self) -> bool {
        use crate::data_transport::{DataMultiTakeHeader, MultiDataHeader};

        match self {
            Self::Value(_, _, update, _)
            | Self::ValueTake(_, _, _, update, _)
            | Self::Static(_, _, update, _)
            | Self::ValueVec(_, _, update, _, _)
            | Self::ValueMap(_, _, update, _, _) => *update,
            Self::Image(_, header, _) => header.update_flag(),
            Self::ImageMulti(_, header) => header.update_flag(),
            Self::Data(_, header) => header.update_flag(),
            Self::DataTake(_, header, _) => header.update_flag(),
            Self::MultiData(_, header) => match header {
                MultiDataHeader::Remove(_, update) | MultiDataHeader::Reset(update) => *update,
                MultiDataHeader::Modify(_, header) => header.update_flag(),
            },
            Self::DataMultiTake(_, header) => match header {
                DataMultiTakeHeader::Remove(_, update) | DataMultiTakeHeader::Reset(update) => {
                    *update
                }
                DataMultiTakeHeader::Modify(_, header, _) => header.update_flag(),
            },
            Self::Update(_) => false,
        }
    }

    /// The parse error for a frame that ends inside this message's payload.
    pub(crate) fn incomplete(&self) -> &'static str {
        match self {
            Self::Value(..) => "Incomplete data for Value message",
            Self::Static(..) => "Incomplete data for Static message",
            Self::ValueTake(..) => "Incomplete data for ValueTake message",
            Self::ValueVec(..) => "Incomplete data for ValueVec message",
            Self::ValueMap(..) => "Incomplete data for Map message",
            Self::Image(..) | Self::ImageMulti(..) => "Incomplete data for Image message",
            Self::Data(..) | Self::MultiData(..) => "Incomplete data for Data/DataStatic message",
            Self::DataTake(..) | Self::DataMultiTake(..) => "Incomplete data for DataTake message",
            Self::Update(_) => "Incomplete Update message",
        }
    }

    #[inline]
    pub fn deserialize(msg: &[u8]) -> Result<(Self, usize), ()> {
        let (header, rest) = postcard::take_from_bytes::<Self>(msg).map_err(|_| ())?;
        Ok((header, msg.len() - rest.len()))
    }
}

// ClientHeader ---------------------------------------------------------------
// ----------------------------------------------------------------------------
#[cfg(feature = "client")]
pub type MessageData = FastVec<32>;

#[derive(Serialize, Deserialize)]
pub(crate) enum ClientHeader {
    Value(u64, u32, bool, u32),
    Signal(u64, u32, u32),
    Ack(u64),
    Message(u32),
    Handshake(u16, Option<u64>, Option<String>),
}

impl ClientHeader {
    #[cfg(feature = "client")]
    pub fn serialize_handshake(
        protocol: u16,
        version: Option<u64>,
        token: Option<String>,
    ) -> FastVec<64> {
        let header = ClientHeader::Handshake(protocol, version, token);
        let data = postcard::to_stdvec(&header).expect("Failed to serialize handshake");
        FastVec::Heap(data)
    }

    /// Bytes of payload that follow this header in the frame.
    #[cfg(any(feature = "server", feature = "python"))]
    pub(crate) fn payload_len(&self) -> u32 {
        match self {
            Self::Value(.., size) | Self::Signal(.., size) | Self::Message(size) => *size,
            Self::Ack(_) | Self::Handshake(..) => 0,
        }
    }

    #[cfg(any(feature = "server", feature = "python"))]
    #[inline]
    pub fn deserialize(msg: &[u8]) -> Result<(Self, usize), ()> {
        let (header, rest) = postcard::take_from_bytes::<Self>(msg).map_err(|_| ())?;
        Ok((header, msg.len() - rest.len()))
    }
}

// Message serialization ---------------------------------------------------------------
// -------------------------------------------------------------------------------------
#[cfg(feature = "client")]
#[inline]
pub(crate) fn to_message<T: Serialize>(value: T) -> MessageData {
    postcard::serialize_with_flavor::<T, MessageData, MessageData>(&value, MessageData::new())
        .expect("Failed to serialize value")
}

#[cfg(feature = "client")]
#[inline]
pub(crate) fn deserialize<T>(data: &[u8]) -> Result<T, String>
where
    T: for<'a> Deserialize<'a>,
{
    postcard::from_bytes(data).map_err(|e| e.to_string())
}

#[cfg(any(feature = "server", feature = "python"))]
#[inline]
pub(crate) fn deserialize_value<T>(data: &[u8]) -> Result<(T, usize), ()>
where
    T: for<'a> Deserialize<'a>,
{
    let (value, new_data) = postcard::take_from_bytes::<T>(data).map_err(|_| ())?;
    Ok((value, data.len() - new_data.len()))
}

#[cfg(feature = "client")]
pub(crate) struct Deserializer<'a> {
    data: &'a [u8],
}

#[cfg(feature = "client")]
impl<'a> Deserializer<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub(crate) fn get<T: for<'b> Deserialize<'b>>(&mut self) -> Result<T, String> {
        let (value, new_data) =
            postcard::take_from_bytes::<T>(self.data).map_err(|e| e.to_string())?;
        self.data = new_data;
        Ok(value)
    }
}

#[inline]
pub(crate) fn serialize<T, const N: usize>(value: &T) -> Result<FastVec<N>, ()>
where
    T: Serialize,
{
    postcard::serialize_with_flavor::<T, FastVec<N>, FastVec<N>>(value, FastVec::new())
        .map_err(|_| ())
}

#[cfg(any(feature = "server", feature = "python"))]
#[inline]
pub(crate) fn serialize_heap<T, const N: usize>(value: &T) -> Result<FastVec<N>, ()>
where
    T: Serialize,
{
    postcard::serialize_with_flavor::<T, FastVec<N>, FastVec<N>>(value, FastVec::new_heap())
        .map_err(|_| ())
}

struct FastVecRef<'a, const N: usize>(&'a mut FastVec<N>);

impl<'a, const N: usize> Flavor for FastVecRef<'a, N> {
    type Output = ();

    #[inline]
    fn try_push(&mut self, data: u8) -> postcard::Result<()> {
        self.0.try_push(data)
    }

    #[inline]
    fn try_extend(&mut self, data: &[u8]) -> postcard::Result<()> {
        self.0.try_extend(data)
    }

    #[inline]
    fn finalize(self) -> postcard::Result<Self::Output> {
        Ok(())
    }
}

#[inline]
pub(crate) fn serialize_to_data<T, const N: usize>(
    value: &T,
    data: &mut FastVec<N>,
) -> Result<(), ()>
where
    T: Serialize,
{
    let data_ref = FastVecRef(data);
    postcard::serialize_with_flavor::<T, FastVecRef<N>, ()>(value, data_ref).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_transport::{ImageHeader, ImageMultiHeader};

    #[test]
    fn value_size_limit_is_inclusive() {
        assert!(check_value_size("root.value", 0).is_ok());
        assert!(check_value_size("root.value", VALUE_MAX_SIZE).is_ok());

        let error = check_value_size("root.value", VALUE_MAX_SIZE + 1).unwrap_err();
        assert!(error.contains("root.value"), "{error}");
        assert!(error.contains(&(VALUE_MAX_SIZE + 1).to_string()), "{error}");
    }

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
            ImageHeader::Fill([1, 1], [0; 4], false),
            0,
        ))
        .unwrap();
        let update: FastVec<32> = serialize(&ServerHeader::Update(0.0)).unwrap();
        let image_multi: FastVec<32> =
            serialize(&ServerHeader::ImageMulti(1, ImageMultiHeader::Reset(false))).unwrap();

        assert_eq!(first_byte(&image), 3);
        assert_eq!(first_byte(&update), 10);
        assert_eq!(first_byte(&image_multi), 11);
    }

    /// Variant indexes are the wire tags of both headers; reordering or inserting
    /// a variant breaks peers built from another version.
    #[test]
    fn header_variant_indexes_are_stable() {
        use crate::collections::{MapHeader, VecHeader};
        use crate::data_transport::{
            DataHeader, DataMultiTakeHeader, DataTakeHeader, DataType, MultiDataHeader,
        };

        let server = [
            ServerHeader::Value(1, 2, false, 0),
            ServerHeader::ValueTake(1, 2, false, false, 0),
            ServerHeader::Static(1, 2, false, 0),
            ServerHeader::Image(1, ImageHeader::Fill([1, 1], [0; 4], false), 0),
            ServerHeader::Data(1, DataHeader::Clear(false)),
            ServerHeader::DataTake(1, DataTakeHeader::Batch(0), false),
            ServerHeader::MultiData(1, MultiDataHeader::Reset(false)),
            ServerHeader::DataMultiTake(1, DataMultiTakeHeader::Reset(false)),
            ServerHeader::ValueVec(1, 2, false, VecHeader::Add, 0),
            ServerHeader::ValueMap(1, 2, false, MapHeader::Set, 0),
            ServerHeader::Update(0.0),
            ServerHeader::ImageMulti(1, ImageMultiHeader::Reset(false)),
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
        use crate::collections::{MapHeader, VecHeader};
        use crate::data_transport::{
            DataHeader, DataMultiTakeHeader, DataTakeHeader, DataType, MultiDataHeader,
            TransportType,
        };
        use crate::image_transport::{ImageSetHeader, ImageType};

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
                DataHeader::All(DataType::U8, TransportType::Set(0), false, 0),
                DataHeader::StartBatch(0, 0),
                DataHeader::Batch(0),
                DataHeader::End(DataType::U8, TransportType::Set(0), false, 0),
                DataHeader::Drain(0, 0, false),
                DataHeader::Clear(false),
            ],
        );
        assert_tags(
            "DataTakeHeader",
            &[
                DataTakeHeader::All(DataType::U8, 0, false, 0),
                DataTakeHeader::StartBatch(0, 0),
                DataTakeHeader::Batch(0),
                DataTakeHeader::End(DataType::U8, 0, false, 0),
            ],
        );
        assert_tags(
            "MultiDataHeader",
            &[
                MultiDataHeader::Remove(0, false),
                MultiDataHeader::Modify(0, DataHeader::Clear(false)),
                MultiDataHeader::Reset(false),
            ],
        );
        assert_tags(
            "DataMultiTakeHeader",
            &[
                DataMultiTakeHeader::Remove(0, false),
                DataMultiTakeHeader::Modify(0, DataTakeHeader::Batch(0), false),
                DataMultiTakeHeader::Reset(false),
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
                ImageSetHeader::All([0; 2], false),
                ImageSetHeader::Start([0; 2], 0),
                ImageSetHeader::Batch(0),
                ImageSetHeader::End(0, false),
            ],
        );
        assert_tags(
            "ImageHeader",
            &[
                ImageHeader::Set(ImageSetHeader::Batch(0), ImageType::Color),
                ImageHeader::Update([0; 4], ImageType::Color, false),
                ImageHeader::Fill([0; 2], [0; 4], false),
            ],
        );
        assert_tags(
            "ImageMultiHeader",
            &[
                ImageMultiHeader::Remove(0, false),
                ImageMultiHeader::Modify(0, ImageHeader::Fill([0; 2], [0; 4], false), 0),
                ImageMultiHeader::Reset(false),
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
        use crate::collections::VecHeader;
        use crate::data_transport::{
            DataHeader, DataMultiTakeHeader, DataTakeHeader, DataType, MultiDataHeader,
            TransportType,
        };
        use crate::image_transport::{ImageSetHeader, ImageType};

        fn bytes<T: Serialize>(value: &T) -> Vec<u8> {
            contents(&serialize::<T, 64>(value).unwrap())
        }

        // postcard: enum tags and integers are varints, `bool` is one byte,
        // `f32` is four little-endian bytes and arrays have no length prefix.
        assert_eq!(bytes(&ServerHeader::Value(5, 7, true, 3)), [0, 5, 7, 1, 3]);
        assert_eq!(
            bytes(&ServerHeader::ValueTake(300, 1, true, false, 2)),
            [1, 0xac, 0x02, 1, 1, 0, 2]
        );
        assert_eq!(
            bytes(&ServerHeader::Data(
                5,
                DataHeader::All(DataType::U16, TransportType::Set(3), true, 6)
            )),
            [4, 5, 0, 1, 0, 3, 1, 6]
        );
        assert_eq!(
            bytes(&ServerHeader::Data(
                5,
                DataHeader::End(DataType::F64, TransportType::Replace(2, 4), false, 32)
            )),
            [4, 5, 3, 9, 2, 2, 4, 0, 32]
        );
        assert_eq!(
            bytes(&ServerHeader::Data(5, DataHeader::Drain(1, 2, true))),
            [4, 5, 4, 1, 2, 1]
        );
        assert_eq!(
            bytes(&ServerHeader::DataTake(
                5,
                DataTakeHeader::All(DataType::U8, 4, true, 4),
                true
            )),
            [5, 5, 0, 0, 4, 1, 4, 1]
        );
        assert_eq!(
            bytes(&ServerHeader::MultiData(
                5,
                MultiDataHeader::Modify(9, DataHeader::Clear(true))
            )),
            [6, 5, 1, 9, 5, 1]
        );
        assert_eq!(
            bytes(&ServerHeader::DataMultiTake(
                5,
                DataMultiTakeHeader::Modify(9, DataTakeHeader::StartBatch(10, 4), false)
            )),
            [7, 5, 1, 9, 1, 10, 4, 0]
        );
        assert_eq!(
            bytes(&ServerHeader::Image(
                5,
                ImageHeader::Set(ImageSetHeader::All([2, 3], true), ImageType::Gray),
                6
            )),
            [3, 5, 0, 0, 2, 3, 1, 2, 6]
        );
        assert_eq!(
            bytes(&ServerHeader::ImageMulti(
                5,
                ImageMultiHeader::Modify(
                    1,
                    ImageHeader::Update([0, 1, 2, 3], ImageType::ColorAlpha, true),
                    24
                )
            )),
            [11, 5, 1, 1, 1, 0, 1, 2, 3, 1, 1, 24]
        );
        assert_eq!(
            bytes(&ServerHeader::ValueVec(
                5,
                7,
                false,
                VecHeader::Remove(2),
                0
            )),
            [8, 5, 7, 0, 3, 2, 0]
        );
        assert_eq!(bytes(&ServerHeader::Update(1.0)), [10, 0, 0, 0x80, 0x3f]);
        assert_eq!(bytes(&ClientHeader::Ack(300)), [2, 0xac, 0x02]);
        assert_eq!(
            bytes(&ClientHeader::Handshake(7, Some(1), None)),
            [4, 7, 1, 1, 0]
        );
    }

    fn contents<const N: usize>(data: &FastVec<N>) -> Vec<u8> {
        match data {
            FastVec::Heap(data) => data.clone(),
            FastVec::Stack(data) => data.as_ref().to_vec(),
        }
    }

    /// A deterministic xorshift generator, so the model test needs no dependency.
    struct Random(u64);

    impl Random {
        fn next(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % bound as u64) as usize
        }

        fn bytes(&mut self, len: usize) -> Vec<u8> {
            (0..len).map(|_| self.next(256) as u8).collect()
        }
    }

    fn other<const M: usize>(bytes: &[u8], heap: bool) -> FastVec<M> {
        if heap {
            FastVec::Heap(bytes.to_vec())
        } else {
            let mut data = FastVec::<M>::new();
            data.try_extend(bytes).unwrap();
            data
        }
    }

    /// Every way of appending must behave like `Vec::extend`, before, at, and
    /// after the stack capacity is exhausted.
    #[test]
    fn fast_vec_appends_match_a_vec_model_across_the_stack_boundary() {
        const N: usize = 8;
        let mut random = Random(0x9e37_79b9_7f4a_7c15);
        for _ in 0..500 {
            let mut data = FastVec::<N>::new();
            let mut model = Vec::new();
            for _ in 0..6 {
                match random.next(5) {
                    0 => {
                        let byte = random.next(256) as u8;
                        data.try_push(byte).unwrap();
                        model.push(byte);
                    }
                    1 => {
                        let len = random.next(2 * N);
                        let bytes = random.bytes(len);
                        data.try_extend(&bytes).unwrap();
                        model.extend(&bytes);
                    }
                    2 | 3 => {
                        // Stack and heap sources of a different capacity.
                        let len = random.next(N / 2 + 1);
                        let bytes = random.bytes(len);
                        let heap = random.next(2) == 0 || bytes.len() > N / 2;
                        data.extend_from_data(&other::<{ N / 2 }>(&bytes, heap));
                        model.extend(&bytes);
                    }
                    _ => {
                        let len = random.next(2 * N);
                        let bytes = random.bytes(len);
                        #[cfg(any(feature = "server", feature = "python"))]
                        data.extend_from_slice(&bytes);
                        #[cfg(not(any(feature = "server", feature = "python")))]
                        data.try_extend(&bytes).unwrap();
                        model.extend(&bytes);
                    }
                }
                assert_eq!(contents(&data), model);
                assert_eq!(data.len(), model.len());
            }
            #[cfg(not(target_arch = "wasm32"))]
            assert_eq!(data.into_bytes().as_ref(), model.as_slice());
        }
    }

    #[test]
    fn fast_vec_switches_to_the_heap_exactly_when_the_stack_is_full() {
        let mut data = FastVec::<4>::new();
        data.try_extend(&[1, 2, 3]).unwrap();
        data.try_push(4).unwrap();
        assert!(
            matches!(data, FastVec::Stack(_)),
            "four bytes fit the stack"
        );
        data.try_push(5).unwrap();
        assert!(matches!(data, FastVec::Heap(_)));
        assert_eq!(contents(&data), [1, 2, 3, 4, 5]);

        let mut data = FastVec::<4>::new();
        data.extend_from_data(&FastVec::<8>::Heap(vec![1, 2, 3, 4]));
        assert!(matches!(data, FastVec::Stack(_)));
        data.extend_from_data(&FastVec::<8>::Heap(vec![5]));
        assert!(matches!(data, FastVec::Heap(_)));
        assert_eq!(contents(&data), [1, 2, 3, 4, 5]);
    }
}
