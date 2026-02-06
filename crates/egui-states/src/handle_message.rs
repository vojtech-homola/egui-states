use bytes::Bytes;

use egui_states_core::collections::{ListHeader, MapHeader};
use egui_states_core::graphs::GraphHeader;
use egui_states_core::image::ImageHeader;
use egui_states_core::serialization::{ClientHeader, FastVec, ServerHeader, serialize_to_data};

use crate::client_base::Client;
use crate::client_states::ValuesList;
use crate::sender::ChannelMessage;

pub(crate) fn parse_to_send(message: ChannelMessage, data: FastVec<64>) -> FastVec<64> {
    match message {
        ChannelMessage::Value(id, signal, msg_data) => {
            let header = ClientHeader::Value(id, signal, msg_data.len() as u32);
            let mut message = serialize_to_data(&header, data).unwrap();
            message.extend_from_data(&msg_data);
            message
        }
        ChannelMessage::Signal(id, msg_data) => {
            let header = ClientHeader::Signal(id, msg_data.len() as u32);
            let mut message = serialize_to_data(&header, data).unwrap();
            message.extend_from_data(&msg_data);
            message
        }
        ChannelMessage::Ack(id) => {
            let header = ClientHeader::Ack(id);
            serialize_to_data(&header, data).unwrap()
        }
    }
}

pub(crate) enum ServerMessage {
    Value(u64, bool, Bytes),
    Static(u64, bool, Bytes),
    Image(u64, bool, ImageHeader, Bytes),
    Graph(u64, bool, GraphHeader, Bytes),
    List(u64, bool, ListHeader, Bytes),
    Map(u64, bool, MapHeader, Bytes),
    Update(f32),
}

pub(crate) struct MessagesParser {
    data: Bytes,
    pointer: usize,
    is_empty: bool,
}

impl MessagesParser {
    pub(crate) fn empty() -> Self {
        Self {
            data: Bytes::new(),
            pointer: 0,
            is_empty: true,
        }
    }

    pub(crate) fn from_bytes(data: Bytes) -> Result<(Self, ServerMessage), &'static str> {
        let mut obj = Self {
            data,
            pointer: 0,
            is_empty: false,
        };
        let message = obj.next_inner()?;
        Ok((obj, message))
    }

    pub(crate) fn next(&mut self) -> Result<Option<ServerMessage>, &'static str> {
        if self.is_empty {
            return Ok(None);
        }

        if self.pointer >= self.data.len() {
            self.is_empty = true;
            return Ok(None);
        }

        let message = self.next_inner()?;
        Ok(Some(message))
    }

    pub fn next_inner(&mut self) -> Result<ServerMessage, &'static str> {
        let (header, size) = ServerHeader::deserialize(&self.data[self.pointer..])
            .map_err(|_| "Failed to deserialize message header")?;
        self.pointer += size;

        let message_data = match header {
            ServerHeader::Value(id, update, size) => {
                let size = size as usize;
                if size + self.pointer > self.data.len() {
                    return Err("Incomplete data for Value message");
                }
                let data = self.data.slice(self.pointer..self.pointer + size);
                self.pointer += size;
                ServerMessage::Value(id, update, data)
            }
            ServerHeader::Static(id, update, size) => {
                let size = size as usize;
                if size + self.pointer > self.data.len() {
                    return Err("Incomplete data for Static message");
                }
                let data = self.data.slice(self.pointer..self.pointer + size);
                self.pointer += size;
                ServerMessage::Static(id, update, data)
            }
            ServerHeader::List(id, update, header, size) => {
                let size = size as usize;
                if size + self.pointer > self.data.len() {
                    return Err("Incomplete data for List message");
                }
                let data = self.data.slice(self.pointer..self.pointer + size);
                self.pointer += size;
                ServerMessage::List(id, update, header, data)
            }
            ServerHeader::Map(id, update, header, size) => {
                let size = size as usize;
                if size + self.pointer > self.data.len() {
                    return Err("Incomplete data for Map message");
                }
                let data = self.data.slice(self.pointer..self.pointer + size);
                self.pointer += size;
                ServerMessage::Map(id, update, header, data)
            }
            ServerHeader::Update(dt) => ServerMessage::Update(dt),
            ServerHeader::Image(id, update, header) => {
                if self.pointer >= self.data.len() {
                    return Err("Incomplete data for Image message");
                }
                let data = self.data.slice(self.pointer..);
                self.is_empty = true;
                ServerMessage::Image(id, update, header, data)
            }
            ServerHeader::Graph(id, update, header) => {
                if self.pointer > self.data.len() {
                    return Err("Incomplete data for Graph message");
                }
                let data =match header {
                    GraphHeader::AddPoints(_, _) | GraphHeader::Set(_, _) => {
                        self.is_empty = true;
                        self.data.slice(self.pointer..)
                    }
                    GraphHeader::Reset | GraphHeader::Remove(_) => {
                        Bytes::new()
                    }
                };
                ServerMessage::Graph(id, update, header, data)
            }
        };

        Ok(message_data)
    }
}

pub(crate) async fn handle_message(
    message: ServerMessage,
    vals: &ValuesList,
    client: &Client,
) -> Result<(), String> {
    let update = match message {
        ServerMessage::Update(t) => {
            client.update(t);
            return Ok(());
        }
        ServerMessage::Value(id, update, data) => {
            match vals.values.get(&id) {
                Some(value) => value.update_value(&data)?,
                None => return Err(format!("Value with id {} not found", id)),
            }
            update
        }
        ServerMessage::Static(id, update, data) => {
            match vals.static_values.get(&id) {
                Some(value) => value.update_value(&data)?,
                None => return Err(format!("Static with id {} not found", id)),
            }
            update
        }
        ServerMessage::Image(id, update, image_header, data) => {
            match vals.images.get(&id) {
                Some(value) => value.update_image(image_header, &data)?,
                None => return Err(format!("Image with id {} not found", id)),
            }
            update
        }
        ServerMessage::List(id, update, list_header, data) => {
            match vals.lists.get(&id) {
                Some(value) => value.update_list(list_header, &data)?,
                None => return Err(format!("List with id {} not found", id)),
            }
            update
        }
        ServerMessage::Map(id, update, map_header, data) => {
            match vals.maps.get(&id) {
                Some(value) => value.update_map(map_header, &data)?,
                None => return Err(format!("Map with id {} not found", id)),
            }
            update
        }
        ServerMessage::Graph(id, update, header, data) => {
            match vals.graphs.get(&id) {
                Some(value) => value.update_graph(header, &data)?,
                None => return Err(format!("Graph with id {} not found", id)),
            }
            update
        }
    };

    if update {
        client.update(0.);
    }

    Ok(())
}
