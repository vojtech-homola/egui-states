use std::io::Write;
use std::net::TcpStream;

pub const HEAD_SIZE: usize = 32;
pub(crate) const MESS_SIZE: usize = 26;
pub(crate) const SIZE_START: usize = MESS_SIZE - 8;

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
