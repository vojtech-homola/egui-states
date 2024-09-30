use std::io::Read;
use std::net::TcpStream;

use egui_pysync_common::commands::CommandMessage;
use egui_pysync_common::transport::{
    self, write_head_data, GraphMessage, ImageMessage, ParseError,
};
use egui_pysync_common::values::ValueMessage;

use crate::dict::WriteDictMessage;
use crate::graphs::WriteGraphMessage;
use crate::list::WriteListMessage;
use crate::signals::ChangedValues;
use crate::states_creator::ValuesList;

pub(crate) enum WriteMessage {
    Value(u32, bool, ValueMessage),
    Static(u32, bool, ValueMessage),
    Image(u32, bool, ImageMessage),
    Dict(u32, bool, Box<dyn WriteDictMessage>),
    List(u32, bool, Box<dyn WriteListMessage>),
    Graph(u32, bool, GraphMessage),
    Command(CommandMessage),
    Terminate,
}

impl WriteMessage {
    #[inline]
    pub fn value(id: u32, update: bool, value: ValueMessage) -> Self {
        WriteMessage::Value(id, update, value)
    }

    #[inline]
    pub fn static_value(id: u32, update: bool, value: ValueMessage) -> Self {
        WriteMessage::Static(id, update, value)
    }

    pub fn dict<T: WriteDictMessage + 'static>(id: u32, update: bool, dict: T) -> Self {
        WriteMessage::Dict(id, update, Box::new(dict))
    }

    pub fn list<T: WriteListMessage + 'static>(id: u32, update: bool, list: T) -> Self {
        WriteMessage::List(id, update, Box::new(list))
    }

    pub fn write_message(
        &self,
        head: &mut [u8],
        stream: &mut std::net::TcpStream,
    ) -> std::io::Result<()> {
        match self {
            WriteMessage::Value(id, update, value) => {
                head[5] = *update as u8;
                let data = value.write_message(head[6..].as_mut());
                write_head_data(head, *id, transport::TYPE_VALUE, data, stream)
            }

            WriteMessage::Static(id, update, value) => {
                head[5] = *update as u8;
                let data = value.write_message(head[6..].as_mut());
                write_head_data(head, *id, transport::TYPE_STATIC, data, stream)
            }

            WriteMessage::Image(id, update, image) => {
                head[0] = transport::TYPE_IMAGE;
                head[1..5].copy_from_slice(&id.to_le_bytes());
                head[5] = *update as u8;
                image.write_message(head, stream)
            }

            WriteMessage::Dict(id, update, dict) => {
                head[5] = *update as u8;
                let data = dict.write_message(head[6..].as_mut());
                write_head_data(head, *id, transport::TYPE_DICT, data, stream)
            }

            WriteMessage::List(id, update, list) => {
                head[5] = *update as u8;
                let data = list.write_message(head[6..].as_mut());
                write_head_data(head, *id, transport::TYPE_LIST, data, stream)
            }

            WriteMessage::Graph(id, update, graph) => {
                head[0] = transport::TYPE_GRAPH;
                head[1..5].copy_from_slice(&id.to_le_bytes());
                head[5] = *update as u8;
                graph.write_message(head, stream)
            }

            WriteMessage::Command(command) => command.write_message(head, stream),

            WriteMessage::Terminate => unreachable!("should not parse Terminate message"),
        }
    }
}

pub(crate) fn read_head(
    head: &mut [u8],
    stream: &mut TcpStream,
) -> Result<Option<CommandMessage>, ParseError> {
    stream
        .read_exact(head)
        .map_err(|e| ParseError::Connection(e))?;

    if head[0] == transport::TYPE_COMMAND {
        CommandMessage::read_message(head, stream).map(|res| Some(res))
    } else {
        Ok(None)
    }
}

pub(crate) fn read_message(
    values: &ValuesList,
    signal: &ChangedValues,
    head: &[u8],
    stream: &mut TcpStream,
) -> Result<(), ParseError> {
    let m_type = head[0];
    match m_type {
        transport::TYPE_VALUE => {
            let id = u32::from_le_bytes(head[1..5].try_into().unwrap());
            let is_signal = head[5] != 0;

            match values.updated.get(&id) {
                Some(val) => {
                    let value = val.update_value(&head[6..], stream)?;
                    if is_signal {
                        signal.set(id, value);
                    }
                    Ok(())
                }
                None => Err(ParseError::Parse(format!("Value with id {} not found", id))),
            }
        }

        transport::TYPE_SIGNAL => {
            let id = u32::from_le_bytes(head[1..5].try_into().unwrap());
            match values.updated.get(&id) {
                Some(val) => {
                    let value = val.update_value(&head[6..], stream)?;
                    signal.set(id, value);
                    Ok(())
                }
                None => Err(ParseError::Parse(format!("Value with id {} not found", id))),
            }
        }

        _ => {
            return Err(ParseError::Parse(format!(
                "Unknown type of the message: {}",
                m_type,
            )))
        }
    }
}
