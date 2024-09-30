use std::collections::HashMap;
use std::io::{self, Write};
use std::net::TcpStream;

pub const HEAD_SIZE: usize = 32;
pub const MESS_SIZE: usize = 26;
pub const SIZE_START: usize = MESS_SIZE - 8;

// message types
pub const TYPE_VALUE: i8 = 16;
pub const TYPE_STATIC: i8 = 32;
pub const TYPE_COMMAND: i8 = 64;
pub const TYPE_IMAGE: i8 = 4;
pub const TYPE_DICT: i8 = 48;
pub const TYPE_LIST: i8 = 96;
pub const TYPE_GRAPH: i8 = 8;
pub const TYPE_SIGNAL: i8 = 12;

/*
Head of the message:

Value:
|1B - type | 4B - u32 value id | 1B - signal / update | = 6B

Static:
|1B - type | 4B - u32 value id | 1B - update | = 6B

Signal:
|1B - type | 4B - u32 value id | 1B - reserve | = 6B

Image:
|1B - type | 4B - u32 value id | 1B - update | = 6B

Dict and List:
|1B - type | 4B - u32 value id | 1B - update | = 6B

Command:
|1B - type | 1B - command |
*/

#[derive(Debug)]
pub enum ParseError {
    Connection(std::io::Error),
    Parse(String),
}

#[inline]
pub fn write_head_data(
    head: &mut [u8],
    id: u32,
    type_: u8,
    data: Option<Vec<u8>>,
    stream: &mut TcpStream,
) -> std::io::Result<()> {
    head[0] = type_;
    head[1..5].copy_from_slice(&id.to_le_bytes());
    stream.write_all(head)?;
    if let Some(data) = data {
        stream.write_all(&data)?;
    }
    Ok(())
}

// dict -----------------------------------------------------------------------

/*
DictMessage

common head:
|1B - type | 4B - u32 value id | 1B - update | = 6B

dict all:
| 8B - u64 count | 8B - u64 size | = 16B
data: | key | value | * count


dict set:
- | 1B - has_data | 4B - u32 size | = 5B
data: | key | value |

- | 1B - has_data | key | value |

dict remove:
- | 1B - has_data | 4B - u32 size | = 5B
data: | key |

- | 1B - has_data | key |
*/

pub const DICT_ALL: u8 = 20;
pub const DICT_SET: u8 = 21;
pub const DICT_REMOVE: u8 = 22;

pub enum DictMessage<K, V> {
    All(HashMap<K, V>),
    Set(K, V),
    Remove(K),
}

// list -----------------------------------------------------------------------
pub const LIST_ALL: u8 = 100;
pub const LIST_SET: u8 = 101;
pub const LIST_ADD: u8 = 102;
pub const LIST_REMOVE: u8 = 103;

pub enum ListMessage<T> {
    All(Vec<T>),
    Set(usize, T),
    Add(T),
    Remove(usize),
}



// graph ----------------------------------------------------------------------
/*
graph head:
| 1B - precision | 1B - operation | 8B - u64 count | 8B - u64 lines | 8B - u64 data size | = 26B

data:
| f32 | f32 | * count
| f64 | f64 | * count
*/
pub const GRAPH_F32: u8 = 60;
pub const GRAPH_F64: u8 = 61;

pub const GRAPH_ADD: u8 = 200;
pub const GRAPH_NEW: u8 = 201;
pub const GRAPH_DELETE: u8 = 202;

#[derive(PartialEq, Copy, Clone)]
pub enum Precision {
    F32,
    F64,
}

pub enum Operation {
    Add,
    New,
    Delete,
}

pub struct GraphMessage {
    pub data: Option<Vec<u8>>,
    pub precision: Precision,
    pub operation: Operation,
    pub count: usize,
    pub lines: usize,
}
