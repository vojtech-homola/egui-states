use heapless;
use heapless::Vec as HVec;
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::net::TcpStream;

use crate::commands::CommandMessage;

pub(crate) const HEAPLESS_SIZE: usize = 32;

// message types
const TYPE_VALUE: u8 = 4;
const TYPE_STATIC: u8 = 8;
const TYPE_SIGNAL: u8 = 10;
const TYPE_COMMAND: u8 = 12;
const TYPE_IMAGE: u8 = 14;
const TYPE_DICT: u8 = 16;
const TYPE_LIST: u8 = 18;
const TYPE_GRAPH: u8 = 20;

pub(crate) enum MessageData {
    Heap(Vec<u8>),
    Stack(HVec<u8, HEAPLESS_SIZE>),
}

#[inline]
pub(crate) fn serialize<T: Serialize>(value: T) -> MessageData {
    match postcard::to_vec::<T, HEAPLESS_SIZE>(&value) {
        Ok(d) => MessageData::Stack(d),
        Err(postcard::Error::SerializeBufferFull) => {
            let data = postcard::to_stdvec(&value).unwrap();
            MessageData::Heap(data)
        }
        Err(e) => panic!("Serialize error: {}", e),
    }
}

#[inline]
pub(crate) fn deserialize<T>(data: MessageData) -> Result<T, postcard::Error>
where
    T: for<'a> Deserialize<'a>,
{
    match data {
        MessageData::Heap(data) => postcard::from_bytes(&data),
        MessageData::Stack(data) => postcard::from_bytes(&data),
    }
}

pub(crate) enum WriteMessage {
    Value(u32, bool, MessageData),
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    Static(u32, bool, MessageData),
    Signal(u32, MessageData),
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    Image(u32, bool, MessageData, Vec<u8>),
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    Dict(u32, bool, MessageData),
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    List(u32, bool, MessageData),
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    Graph(u32, bool, MessageData, Option<Vec<u8>>),
    Command(CommandMessage),
    Terminate,
}

impl WriteMessage {
    pub fn ack(id: u32) -> Self {
        WriteMessage::Command(CommandMessage::Ack(id))
    }
}

pub(crate) enum ReadMessage {
    Value(u32, bool, MessageData),
    Static(u32, bool, MessageData),
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    Signal(u32, MessageData),
    Image(u32, bool, MessageData),
    Dict(u32, bool, MessageData),
    List(u32, bool, MessageData),
    Graph(u32, bool, MessageData),
    Command(CommandMessage),
}

#[cfg(feature = "server")]
impl ReadMessage {
    pub fn to_str(&self) -> &'static str {
        match self {
            Self::Value(_, _, _) => "Value",
            Self::Static(_, _, _) => "Static",
            Self::Signal(_, _) => "Signal",
            Self::Image(_, _, _) => "Image",
            Self::Dict(_, _, _) => "Dict",
            Self::List(_, _, _) => "List",
            Self::Graph(_, _, _) => "Graph",
            Self::Command(_) => "Command",
        }
    }
}

fn write_data(
    head: &mut [u8],
    data: &MessageData,
    stream: &mut TcpStream,
    add_size: Option<usize>,
) -> std::io::Result<()> {
    match data {
        MessageData::Heap(data) => {
            match add_size {
                Some(size) => {
                    head[0..4].copy_from_slice(&((data.len() + size) as u32).to_le_bytes());
                }
                None => {
                    head[0..4].copy_from_slice(&(data.len() as u32).to_le_bytes());
                }
            }
            stream.write_all(head)?;
            stream.write_all(data)
        }
        MessageData::Stack(data) => {
            match add_size {
                Some(size) => {
                    head[0..4].copy_from_slice(&((data.len() + size) as u32).to_le_bytes());
                }
                None => {
                    head[0..4].copy_from_slice(&(data.len() as u32).to_le_bytes());
                }
            }
            stream.write_all(head)?;
            stream.write_all(data)
        }
    }
}

pub(crate) fn write_message(message: WriteMessage, stream: &mut TcpStream) -> std::io::Result<()> {
    let mut head = [0u8; 10];
    match message {
        WriteMessage::Value(id, flag, data) => {
            head[4] = TYPE_VALUE;
            head[5] = flag as u8;
            head[6..10].copy_from_slice(&id.to_le_bytes());
            write_data(&mut head, &data, stream, None)
        }
        WriteMessage::Signal(id, data) => {
            head[4] = TYPE_SIGNAL;
            head[6..10].copy_from_slice(&id.to_le_bytes());
            write_data(&mut head, &data, stream, None)
        }
        WriteMessage::Static(id, flag, data) => {
            head[4] = TYPE_STATIC;
            head[5] = flag as u8;
            head[6..10].copy_from_slice(&id.to_le_bytes());
            write_data(&mut head, &data, stream, None)
        }
        WriteMessage::Dict(id, flag, data) => {
            head[4] = TYPE_DICT;
            head[5] = flag as u8;
            head[6..10].copy_from_slice(&id.to_le_bytes());
            write_data(&mut head, &data, stream, None)
        }
        WriteMessage::List(id, flag, data) => {
            head[4] = TYPE_LIST;
            head[5] = flag as u8;
            head[6..10].copy_from_slice(&id.to_le_bytes());
            write_data(&mut head, &data, stream, None)
        }
        WriteMessage::Image(id, flag, info, data) => {
            head[4] = TYPE_IMAGE;
            head[5] = flag as u8;
            head[6..10].copy_from_slice(&id.to_le_bytes());
            write_data(&mut head, &info, stream, Some(data.len()))?;
            stream.write_all(&data)
        }
        WriteMessage::Graph(id, flag, data, graph_data) => {
            head[4] = TYPE_GRAPH;
            head[5] = flag as u8;
            head[6..10].copy_from_slice(&id.to_le_bytes());
            let size = match &graph_data {
                Some(data) => Some(data.len()),
                None => None,
            };
            write_data(&mut head, &data, stream, size)?;
            if let Some(graph_data) = graph_data {
                stream.write_all(&graph_data)
            } else {
                Ok(())
            }
        }
        WriteMessage::Command(command) => {
            head[4] = TYPE_COMMAND;
            let data = serialize(&command);
            write_data(&mut head, &data, stream, None)
        }
        WriteMessage::Terminate => {
            unreachable!("Terminate message should not be written");
        }
    }
}

pub(crate) fn read_message(stream: &mut TcpStream) -> Result<ReadMessage, io::Error> {
    let mut head = [0u8; 10];
    stream.read_exact(&mut head)?;

    let message_size = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let message_type = head[4];
    let flag = head[5] != 0;
    let id = u32::from_le_bytes([head[6], head[7], head[8], head[9]]);

    let data = if message_size > HEAPLESS_SIZE {
        let mut data = Vec::with_capacity(message_size);
        unsafe { data.set_len(message_size) };
        stream.read_exact(&mut data)?;
        MessageData::Heap(data)
    } else {
        let mut data: heapless::Vec<u8, HEAPLESS_SIZE> = heapless::Vec::new();
        unsafe { data.set_len(message_size) };
        stream.read_exact(&mut data)?;
        MessageData::Stack(data)
    };

    match message_type {
        TYPE_VALUE => Ok(ReadMessage::Value(id, flag, data)),
        TYPE_STATIC => Ok(ReadMessage::Static(id, flag, data)),
        TYPE_SIGNAL => Ok(ReadMessage::Signal(id, data)),
        TYPE_LIST => Ok(ReadMessage::List(id, flag, data)),
        TYPE_DICT => Ok(ReadMessage::Dict(id, flag, data)),
        TYPE_GRAPH => Ok(ReadMessage::Graph(id, flag, data)),
        TYPE_IMAGE => Ok(ReadMessage::Image(id, flag, data)),
        TYPE_COMMAND => {
            let command = deserialize(data).unwrap(); // TODO: handle error
            Ok(ReadMessage::Command(command))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Unknown message type",
        )),
    }
}
