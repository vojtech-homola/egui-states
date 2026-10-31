use postcard::ser_flavors::Flavor;
use serde::{Deserialize, Serialize};

use crate::collections::{MapHeader, VecHeader};
use crate::data_transport;
use crate::image_transport::{ImageHeader, ImageMultiHeader};

// TODO: make these constants configurable
pub(crate) const VALUE_MAX_SIZE: usize = 1024 * 1024; // 1 MB
pub(crate) const MSG_SIZE_THRESHOLD: usize = 1024 * 1024 * 10; // 10 MB
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
