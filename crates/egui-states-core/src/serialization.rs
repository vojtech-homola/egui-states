use postcard::{
    ser_flavors::{Flavor, StdVec},
    serialize_with_flavor,
};
use serde::{Deserialize, Serialize};

// message types
pub const TYPE_VALUE: u8 = 4;
pub const TYPE_STATIC: u8 = 8;
pub const TYPE_SIGNAL: u8 = 10;
pub const TYPE_CONTROL: u8 = 12;
pub const TYPE_IMAGE: u8 = 14;
pub const TYPE_DICT: u8 = 16;
pub const TYPE_LIST: u8 = 18;
pub const TYPE_GRAPH: u8 = 20;

pub const HEAPLESS_SIZE: usize = 32;

pub enum MessageData {
    Heap(Vec<u8>),
    Stack([u8; HEAPLESS_SIZE], usize),
}

impl MessageData {
    pub fn to_vec(self) -> Vec<u8> {
        match self {
            MessageData::Heap(vec) => vec,
            MessageData::Stack(arr, len) => {
                arr[0..len].to_vec()
            }
        }
    }
}

#[inline]
pub fn serialize<T: Serialize>(id: u32, value: T, value_type: u8) -> MessageData {
    let mut stack_data: [u8; HEAPLESS_SIZE] = [0; HEAPLESS_SIZE];
    stack_data[0] = value_type;
    stack_data[1..5].copy_from_slice(&id.to_le_bytes());

    let len = match postcard::to_slice(&value, stack_data[5..].as_mut()) {
        Ok(d) => Some(d.len() + 5),
        Err(postcard::Error::SerializeBufferFull) => None,
        Err(e) => panic!("Serialize error: {}", e),
    };

    match len {
        Some(l) => MessageData::Stack(stack_data, l),
        None => {
            let mut data = StdVec::new();
            unsafe { data.try_extend(&stack_data[0..5]).unwrap_unchecked() };
            let data = serialize_with_flavor::<T, StdVec, Vec<u8>>(&value, data).unwrap();
            MessageData::Heap(data)
        }
    }
}

#[inline]
pub fn serialize_vec<T: Serialize>(id: u32, value: T, value_type: u8) -> Vec<u8> {
    let mut head = [0; 5];
    head[0] = value_type;
    head[1..5].copy_from_slice(&id.to_le_bytes());
    let mut data = StdVec::new();
    unsafe { data.try_extend(&head).unwrap_unchecked() };
    serialize_with_flavor::<T, StdVec, Vec<u8>>(&value, data).unwrap()
}

#[inline]
pub fn deserialize<T>(data: &[u8]) -> Result<T, String>
where
    T: for<'a> Deserialize<'a>,
{
    postcard::from_bytes(data).map_err(|e| e.to_string())
}
