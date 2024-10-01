use std::io::Read;
use std::net::TcpStream;

use egui_pysync_common::commands::CommandMessage;
use egui_pysync_common::transport::{self, ParseError};
use egui_pysync_common::values::ValueMessage;
use egui_pysync_common::image::ImageMessage;
use egui_pysync_common::graphs::GraphsMessage;

// use crate::image::read_image_message;
use crate::states_creator::ValuesList;

pub(crate) enum WriteMessage {
    Value(u32, bool, ValueMessage),
    Signal(u32, ValueMessage),
    Command(CommandMessage),
    Terminate,
}

impl WriteMessage {
    #[inline]
    pub fn value(id: u32, signal: bool, value: ValueMessage) -> Self {
        WriteMessage::Value(id, signal, value)
    }

    #[inline]
    pub fn signal(id: u32, value: ValueMessage) -> Self {
        WriteMessage::Signal(id, value)
    }

    pub fn ack(id: u32) -> Self {
        WriteMessage::Command(CommandMessage::Ack(id))
    }

    pub fn write_message(
        &self,
        head: &mut [u8],
        stream: &mut std::net::TcpStream,
    ) -> std::io::Result<()> {
        match self {
            WriteMessage::Value(id, signal, value) => {
                head[5] = *signal as u8;
                let data = value.write_message(head[6..].as_mut());
                transport::write_head_data(head, *id, transport::TYPE_VALUE, data, stream)
            }

            WriteMessage::Signal(id, value) => {
                let data = value.write_message(head[6..].as_mut());
                transport::write_head_data(head, *id, transport::TYPE_SIGNAL, data, stream)
            }

            WriteMessage::Command(command) => command.write_message(head, stream),

            WriteMessage::Terminate => unreachable!("should not parse Terminate message"),
        }
    }
}

pub(crate) enum ReadResult {
    Update(bool),
    Command(CommandMessage),
}

pub(crate) enum ReadMessage<'a> {
    Value(u32, bool, &'a [u8], Option<Vec<u8>>),
    Static(u32, bool, &'a [u8], Option<Vec<u8>>),
    Image(u32, bool, ImageMessage),
    Dict(u32, bool, &'a [u8], Option<Vec<u8>>),
    List(u32, bool, &'a [u8], Option<Vec<u8>>),
    Graph(u32, bool, GraphsMessage),
    Command(CommandMessage),
}

pub(crate) fn read_message(
    values: &ValuesList,
    head: &mut [u8],
    stream: &mut TcpStream,
) -> Result<ReadResult, ParseError> {
    stream
        .read_exact(head)
        .map_err(|e| ParseError::Connection(e))?;

    let m_type = head[0];

    match m_type {
        transport::TYPE_VALUE => {
            let id = u32::from_le_bytes(head[1..5].try_into().unwrap());
            let update = head[5] != 0;

            match values.values.get(&id) {
                Some(val) => val.update_value(&head[6..], stream),
                None => Err(ParseError::Parse(format!("Value with id {} not found", id))),
            }
            .map(|_| ReadResult::Update(update))
        }

        transport::TYPE_STATIC => {
            let id = u32::from_le_bytes(head[1..5].try_into().unwrap());
            let update = head[5] != 0;

            match values.static_values.get(&id) {
                Some(val) => val.update_value(&head[6..], stream),
                None => Err(ParseError::Parse(format!(
                    "Static value with id {} not found",
                    id
                ))),
            }
            .map(|_| ReadResult::Update(update))
        }

        transport::TYPE_IMAGE => {
            let id = u32::from_le_bytes(head[1..5].try_into().unwrap());
            let update = head[5] != 0;
            let image = read_image_message(head, stream)?;

            match values.images.get(&id) {
                Some(val) => val.update_image(image).map_err(|e| ParseError::Parse(e)),
                None => Err(ParseError::Parse(format!("Image with id {} not found", id))),
            }
            .map(|_| ReadResult::Update(update))
        }

        transport::TYPE_DICT => {
            let id = u32::from_le_bytes(head[1..5].try_into().unwrap());
            let update = head[5] != 0;

            match values.dicts.get(&id) {
                Some(val) => val.update_dict(&head[6..], stream),
                None => Err(ParseError::Parse(format!(
                    "Dict value with id {} not found",
                    id
                ))),
            }
            .map(|_| ReadResult::Update(update))
        }

        transport::TYPE_LIST => {
            let id = u32::from_le_bytes(head[1..5].try_into().unwrap());
            let update = head[5] != 0;

            match values.lists.get(&id) {
                Some(val) => val.update_list(&head[6..], stream),
                None => Err(ParseError::Parse(format!(
                    "List value with id {} not found",
                    id
                ))),
            }
            .map(|_| ReadResult::Update(update))
        }

        transport::TYPE_GRAPH => {
            let id = u32::from_le_bytes(head[1..5].try_into().unwrap());
            let update = head[5] != 0;

            match values.graphs.get(&id) {
                Some(val) => val.update_graph(&head[6..0], stream),
                None => Err(ParseError::Parse(format!(
                    "Graph value with id {} not found",
                    id
                ))),
            }
            .map(|_| ReadResult::Update(update))
        }

        transport::TYPE_COMMAND => {
            let command = CommandMessage::read_message(head, stream)?;
            Ok(ReadResult::Command(command))
        }

        _ => {
            return Err(ParseError::Parse(format!(
                "Unknown type of the message: {}",
                m_type,
            )))
        }
    }
}
// }
