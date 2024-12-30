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

/*
Head of the message:

Value:
|1B - type / flag | 3B - u32 value id | = 4B

Static:
|1B - type / flag | 3B - u32 value id | = 4B

Signal:
|1B - type / flag | 3B - u32 value id | = 4B

Image:
|1B - type / flag | 3B - u32 value id | = 4B

Dict and List:
|1B - type / flag | 3B - u32 value id | = 4B

Command:
|1B - type | 1B - command |
*/

// pub fn write_message_old(
//     head: &[u8],
//     data: Option<Vec<u8>>,
//     stream: &mut TcpStream,
// ) -> std::io::Result<()> {
//     stream.write_all(head)?;
//     if let Some(data) = data {
//         stream.write_all(&data)?;
//     }
//     Ok(())
// }

// pub fn read_message_old(
//     head: &mut [u8],
//     stream: &mut TcpStream,
// ) -> Result<(i8, Option<Vec<u8>>), io::Error> {
//     stream.read_exact(head)?;
//     let type_ = head[0] as i8;
//     let has_data = type_.is_negative();
//     let type_ = type_.abs();

//     let data = match has_data {
//         true => {
//             let size = u32::from_le_bytes(head[SIZE_START..].try_into().unwrap()) as usize;
//             let mut data = vec![0u8; size];
//             stream.read_exact(&mut data)?;
//             Some(data)
//         }
//         false => None,
//     };

//     Ok((type_, data))
// }

pub trait WriteMessageDyn: Send + Sync + 'static {
    fn write_message(&self) -> MessageData;
}

pub enum WriteMessage {
    Value(u32, bool, MessageData),
    Static(u32, bool, MessageData),
    Signal(u32, MessageData),
    Image(u32, bool, MessageData, Vec<u8>),
    Dict(u32, bool, MessageData),
    List(u32, bool, MessageData),
    Graph(u32, bool, MessageData, Option<Vec<u8>>),
    Command(CommandMessage),
    Terminate,
}

impl WriteMessage {
    pub fn ack(id: u32) -> Self {
        WriteMessage::Command(CommandMessage::Ack(id))
    }

    // pub fn parse(self, head: &mut [u8]) -> Option<Vec<u8>> {
    //     if let WriteMessage::Command(command) = self {
    //         let data = command.write_message(&mut head[1..]);
    //         match data {
    //             Some(ref data) => {
    //                 head[0] = -TYPE_COMMAND as u8;
    //                 let size = data.len() as u32;
    //                 head[SIZE_START..].copy_from_slice(&size.to_le_bytes());
    //             }
    //             None => head[0] = TYPE_COMMAND as u8,
    //         }
    //         return data;
    //     }

    //     let (id, flag, mut type_, data) = match self {
    //         Self::Value(id, update_signal, message) => {
    //             let data = message.write_message(&mut head[4..]);
    //             (id, update_signal, TYPE_VALUE, data)
    //         }

    //         Self::Static(id, update, message) => {
    //             let data = message.write_message(&mut head[4..]);
    //             (id, update, TYPE_STATIC, data)
    //         }

    //         Self::Signal(id, message) => {
    //             let data = message.write_message(&mut head[4..]);
    //             (id, false, TYPE_SIGNAL, data)
    //         }

    //         Self::Image(id, update, message) => {
    //             let data = message.write_message(&mut head[4..]);
    //             (id, update, TYPE_IMAGE, Some(data))
    //         }

    //         Self::Dict(id, update, dict) => {
    //             let data = dict.write_message(&mut head[4..]);
    //             (id, update, TYPE_DICT, data)
    //         }

    //         Self::List(id, update, list) => {
    //             let data = list.write_message(&mut head[4..]);
    //             (id, update, TYPE_LIST, data)
    //         }

    //         Self::Graph(id, update, message) => {
    //             let data = message.write_message(&mut head[4..]);
    //             (id, update, TYPE_GRAPH, data)
    //         }

    //         Self::Terminate | Self::Command(_) => {
    //             unreachable!("should not parse Terminate message")
    //         }
    //     };

    //     if flag {
    //         type_ += 64;
    //     }

    //     if let Some(ref data) = data {
    //         type_ = -type_;
    //         let size = data.len() as u32;
    //         head[SIZE_START..].copy_from_slice(&size.to_le_bytes());
    //     }

    //     head[0] = type_ as u8;
    //     head[1..4].copy_from_slice(&id.to_le_bytes()[0..3]);
    //     data
    // }
}

pub enum ReadMessage {
    Value(u32, bool, MessageData),
    Static(u32, bool, MessageData),
    Signal(u32, MessageData),
    Image(u32, bool, MessageData),
    Dict(u32, bool, MessageData),
    List(u32, bool, MessageData),
    Graph(u32, bool, MessageData),
    Command(CommandMessage),
}

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

// impl<'a> ReadMessage<'a> {
//     pub fn parse(
//         head: &'a [u8],
//         mut message_type: i8,
//         data: Option<Vec<u8>>,
//     ) -> Result<ReadMessage<'a>, String> {
//         if message_type == TYPE_COMMAND {
//             let command = CommandMessage::read_message(&head[1..], data)?;
//             return Ok(ReadMessage::Command(command));
//         }

//         let id = u32::from_le_bytes([head[1], head[2], head[3], 0]);
//         let update = if message_type > 63 {
//             message_type -= 64;
//             true
//         } else {
//             false
//         };

//         match message_type {
//             TYPE_VALUE => Ok(ReadMessage::Value(id, update, &head[4..], data)),
//             TYPE_STATIC => Ok(ReadMessage::Static(id, update, &head[4..], data)),
//             TYPE_SIGNAL => Ok(ReadMessage::Signal(id, &head[4..], data)),
//             TYPE_IMAGE => {
//                 let image = ImageMessage::read_message(&head[4..], data)?;
//                 Ok(ReadMessage::Image(id, update, image))
//             }
//             TYPE_DICT => Ok(ReadMessage::Dict(id, update, &head[4..], data)),
//             TYPE_LIST => Ok(ReadMessage::List(id, update, &head[4..], data)),
//             TYPE_GRAPH => Ok(ReadMessage::Graph(id, update, &head[4..], data)),
//             _ => Err(format!("Unknown message type: {}", message_type)),
//         }
//     }
// }

fn write_data(head: &mut [u8], data: &MessageData, stream: &mut TcpStream) -> std::io::Result<()> {
    match data {
        MessageData::Heap(data) => {
            head[0..4].copy_from_slice(&(data.len() as u32).to_le_bytes());
            stream.write_all(head)?;
            stream.write_all(data)
        }
        MessageData::Stack(data) => {
            head[0..4].copy_from_slice(&(data.len() as u32).to_le_bytes());
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
            head[6..9].copy_from_slice(&id.to_le_bytes());
            write_data(&mut head, &data, stream)
        }
        WriteMessage::Signal(id, data) => {
            head[4] = TYPE_SIGNAL;
            head[5..8].copy_from_slice(&id.to_le_bytes());
            write_data(&mut head, &data, stream)
        }
        WriteMessage::Static(id, flag, data) => {
            head[4] = TYPE_STATIC;
            head[5] = flag as u8;
            head[6..9].copy_from_slice(&id.to_le_bytes());
            write_data(&mut head, &data, stream)
        }
        WriteMessage::Dict(id, flag, data) => {
            head[4] = TYPE_DICT;
            head[5] = flag as u8;
            head[6..9].copy_from_slice(&id.to_le_bytes());
            write_data(&mut head, &data, stream)
        }
        WriteMessage::List(id, flag, data) => {
            head[4] = TYPE_LIST;
            head[5] = flag as u8;
            head[6..9].copy_from_slice(&id.to_le_bytes());
            write_data(&mut head, &data, stream)
        }
        WriteMessage::Image(id, flag, info, data) => {
            head[4] = TYPE_IMAGE;
            head[5] = flag as u8;
            head[6..9].copy_from_slice(&id.to_le_bytes());
            write_data(&mut head, &info, stream)?;
            write_data(&mut head, &MessageData::Heap(data), stream)
        }
        WriteMessage::Graph(id, flag, data, graph_data) => {
            head[4] = TYPE_GRAPH;
            head[5] = flag as u8;
            head[6..9].copy_from_slice(&id.to_le_bytes());
            write_data(&mut head, &data, stream)?;
            if let Some(graph_data) = graph_data {
                write_data(&mut head, &MessageData::Heap(graph_data), stream)
            } else {
                Ok(())
            }
        }
        WriteMessage::Command(command) => {
            head[4] = TYPE_COMMAND;
            let data = serialize(&command);
            write_data(&mut head, &data, stream)
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
