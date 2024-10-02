use std::io::{self, Read, Write};
use std::net::TcpStream;

use crate::commands::CommandMessage;
use crate::dict::WriteDictMessage;
use crate::graphs::GraphMessage;
use crate::image::{HistogramMessage, ImageMessage};
use crate::list::WriteListMessage;
use crate::values::ValueMessage;

pub const HEAD_SIZE: usize = 32;
pub(crate) const MESS_SIZE: usize = 26;
pub(crate) const SIZE_START: usize = MESS_SIZE - 8;

// message types
const TYPE_VALUE: u8 = 16;
const TYPE_STATIC: u8 = 32;
const TYPE_SIGNAL: u8 = 12;
const TYPE_COMMAND: u8 = 64;
const TYPE_IMAGE: u8 = 4;
const TYPE_HISTOGRAM: u8 = 24;
const TYPE_DICT: u8 = 48;
const TYPE_LIST: u8 = 96;
const TYPE_GRAPH: u8 = 8;

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

pub fn write_message(
    head: &[u8],
    data: Option<Vec<u8>>,
    stream: &mut TcpStream,
) -> std::io::Result<()> {
    stream.write_all(head)?;
    if let Some(data) = data {
        stream.write_all(&data)?;
    }
    Ok(())
}

pub fn read_message(
    head: &mut [u8],
    stream: &mut TcpStream,
) -> Result<(u8, Option<Vec<u8>>), io::Error> {
    stream.read_exact(head)?;
    let type_ = head[0] as i8;
    let has_data = type_.is_negative();
    let type_ = type_.abs() as u8;

    let data = match has_data {
        true => {
            const SIZE_START: usize = HEAD_SIZE - 8;
            let size = u64::from_le_bytes(head[SIZE_START..].try_into().unwrap()) as usize;
            let mut data = vec![0u8; size];
            stream.read_exact(&mut data)?;
            Some(data)
        }
        false => None,
    };

    Ok((type_, data))
}

pub enum WriteMessage {
    Value(u32, bool, ValueMessage),
    Static(u32, bool, ValueMessage),
    Signal(u32, ValueMessage),
    Image(u32, bool, ImageMessage),
    Histogram(u32, bool, HistogramMessage),
    Dict(u32, bool, Box<dyn WriteDictMessage>),
    List(u32, bool, Box<dyn WriteListMessage>),
    Graph(u32, bool, GraphMessage),
    Command(CommandMessage),
    Terminate,
}

impl WriteMessage {
    pub fn ack(id: u32) -> Self {
        WriteMessage::Command(CommandMessage::Ack(id))
    }

    pub fn list(id: u32, update: bool, list: impl WriteListMessage) -> Self {
        WriteMessage::List(id, update, Box::new(list))
    }

    pub fn dict(id: u32, update: bool, dict: impl WriteDictMessage) -> Self {
        WriteMessage::Dict(id, update, Box::new(dict))
    }

    pub fn parse(self, head: &mut [u8]) -> Option<Vec<u8>> {
        if let WriteMessage::Command(command) = self {
            return command.write_message(head);
        }

        let (id, flag, type_, data) = match self {
            Self::Value(id, update_signal, message) => {
                let data = message.write_message(&mut head[6..]);
                (id, update_signal, TYPE_VALUE, data)
            }

            Self::Static(id, update, message) => {
                let data = message.write_message(&mut head[6..]);
                (id, update, TYPE_STATIC, data)
            }

            Self::Signal(id, message) => {
                let data = message.write_message(&mut head[6..]);
                (id, false, TYPE_SIGNAL, data)
            }

            Self::Image(id, update, message) => {
                let data = message.write_message(head);
                (id, update, TYPE_IMAGE, Some(data))
            }

            Self::Histogram(id, update, message) => {
                let data = message.write_message(head);
                (id, update, TYPE_HISTOGRAM, data)
            }

            Self::Dict(id, update, dict) => {
                let data = dict.write_message(&mut head[6..]);
                (id, update, TYPE_DICT, data)
            }

            Self::List(id, update, list) => {
                let data = list.write_message(&mut head[6..]);
                (id, update, TYPE_LIST, data)
            }

            Self::Graph(id, update, message) => {
                let data = message.write_message(head);
                (id, update, TYPE_GRAPH, data)
            }

            Self::Terminate | Self::Command(_) => {
                unreachable!("should not parse Terminate message")
            }
        };

        let mut type_ = type_ as i8;
        if data.is_some() {
            type_ = -type_;
        }

        head[0] = type_ as u8;
        head[1..5].copy_from_slice(&id.to_le_bytes());
        head[5] = flag as u8;

        data
    }
}

pub enum ReadMessage<'a> {
    Value(u32, bool, &'a [u8], Option<Vec<u8>>),
    Static(u32, bool, &'a [u8], Option<Vec<u8>>),
    Signal(u32, &'a [u8], Option<Vec<u8>>),
    Image(u32, bool, ImageMessage),
    Histogram(u32, bool, HistogramMessage),
    Dict(u32, bool, &'a [u8], Option<Vec<u8>>),
    List(u32, bool, &'a [u8], Option<Vec<u8>>),
    Graph(u32, bool, GraphMessage),
    Command(CommandMessage),
}

impl<'a> ReadMessage<'a> {
    pub fn parse(
        head: &'a [u8],
        message_type: u8,
        data: Option<Vec<u8>>,
    ) -> Result<ReadMessage<'a>, String> {
        if message_type == TYPE_COMMAND {
            let command = CommandMessage::read_message(&head[1..], data)?;
            return Ok(ReadMessage::Command(command));
        }

        let id = u32::from_le_bytes(head[1..5].try_into().unwrap());
        let update = head[5] != 0;

        match message_type {
            TYPE_VALUE => Ok(ReadMessage::Value(id, update, &head[6..], data)),
            TYPE_STATIC => Ok(ReadMessage::Static(id, update, &head[6..], data)),
            TYPE_SIGNAL => Ok(ReadMessage::Signal(id, &head[6..], data)),
            TYPE_IMAGE => {
                let image = ImageMessage::read_message(head, data)?;
                Ok(ReadMessage::Image(id, update, image))
            }
            TYPE_HISTOGRAM => {
                let histogram = HistogramMessage::read_message(head, data)?;
                Ok(ReadMessage::Histogram(id, update, histogram))
            }
            TYPE_DICT => Ok(ReadMessage::Dict(id, update, &head[6..], data)),
            TYPE_LIST => Ok(ReadMessage::List(id, update, &head[6..], data)),
            TYPE_GRAPH => {
                let graph = GraphMessage::read_message(head, data)?;
                Ok(ReadMessage::Graph(id, update, graph))
            }
            _ => Err(format!("Unknown message type: {}", head[0])),
        }
    }
}

// #[derive(Debug)]
// pub enum ParseError {
//     Connection(std::io::Error),
//     Parse(String),
// }

// #[inline]
// pub fn write_head_data(
//     head: &mut [u8],
//     id: u32,
//     type_: u8,
//     data: Option<Vec<u8>>,
//     stream: &mut TcpStream,
// ) -> std::io::Result<()> {
//     head[0] = type_;
//     head[1..5].copy_from_slice(&id.to_le_bytes());
//     stream.write_all(head)?;
//     if let Some(data) = data {
//         stream.write_all(&data)?;
//     }
//     Ok(())
// }
