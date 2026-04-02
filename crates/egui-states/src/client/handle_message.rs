use bytes::Bytes;

use crate::client::client::Client;
use crate::client::sender::ChannelMessage;
use crate::client::states_creator::ValuesList;
use crate::collections::{MapHeader, VecHeader};
use crate::graphs::GraphHeader;
use crate::image::ImageHeader;
use crate::serialization::{ClientHeader, FastVec, ServerHeader, serialize_to_data};

pub(crate) fn parse_to_send(message: ChannelMessage, data: &mut FastVec<64>) {
    match message {
        ChannelMessage::Value(id, type_id, signal, msg_data) => {
            let header = ClientHeader::Value(id, type_id, signal, msg_data.len() as u32);
            serialize_to_data(&header, data).unwrap();
            data.extend_from_data(&msg_data);
        }
        ChannelMessage::Signal(id, type_id, msg_data) => {
            let header = ClientHeader::Signal(id, type_id, msg_data.len() as u32);
            serialize_to_data(&header, data).unwrap();
            data.extend_from_data(&msg_data);
        }
        ChannelMessage::Ack(id) => {
            let header = ClientHeader::Ack(id);
            serialize_to_data(&header, data).unwrap();
        }
    }
}

pub(crate) enum ServerMessage {
    Value(u64, u32, bool, Bytes),
    ValueTake(u64, u32, bool, bool, Bytes),
    Static(u64, u32, bool, Bytes),
    Image(u64, bool, ImageHeader, Bytes),
    Graph(u64, bool, GraphHeader, Bytes),
    ValueVec(u64, u32, bool, VecHeader, Bytes),
    ValueMap(u64, u32, bool, MapHeader, Bytes),
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
            ServerHeader::Value(id, type_id, update, size) => {
                let size = size as usize;
                if size + self.pointer > self.data.len() {
                    return Err("Incomplete data for Value message");
                }
                let data = self.data.slice(self.pointer..self.pointer + size);
                self.pointer += size;
                ServerMessage::Value(id, type_id, update, data)
            }
            ServerHeader::Static(id, type_id, update, size) => {
                let size = size as usize;
                if size + self.pointer > self.data.len() {
                    return Err("Incomplete data for Static message");
                }
                let data = self.data.slice(self.pointer..self.pointer + size);
                self.pointer += size;
                ServerMessage::Static(id, type_id, update, data)
            }
            ServerHeader::ValueTake(id, type_id, blocking, update, size) => {
                let size = size as usize;
                if size + self.pointer > self.data.len() {
                    return Err("Incomplete data for ValueTake message");
                }
                let data = self.data.slice(self.pointer..self.pointer + size);
                self.pointer += size;
                ServerMessage::ValueTake(id, type_id, blocking, update, data)
            }
            ServerHeader::ValueVec(id, type_id, update, header, size) => {
                let size = size as usize;
                if size + self.pointer > self.data.len() {
                    return Err("Incomplete data for ValueVec message");
                }
                let data = self.data.slice(self.pointer..self.pointer + size);
                self.pointer += size;
                ServerMessage::ValueVec(id, type_id, update, header, data)
            }
            ServerHeader::ValueMapMap(id, type_id, update, header, size) => {
                let size = size as usize;
                if size + self.pointer > self.data.len() {
                    return Err("Incomplete data for Map message");
                }
                let data = self.data.slice(self.pointer..self.pointer + size);
                self.pointer += size;
                ServerMessage::ValueMap(id, type_id, update, header, data)
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
                let data = match header {
                    GraphHeader::AddPoints(_, _) | GraphHeader::Set(_, _) => {
                        self.is_empty = true;
                        self.data.slice(self.pointer..)
                    }
                    GraphHeader::Reset | GraphHeader::Remove(_) => Bytes::new(),
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
        ServerMessage::Value(id, type_id, update, data) => {
            match vals.values.get(&id) {
                Some(value) => value.update_value(type_id, &data)?,
                None => return Err(format!("Value with id {} not found", id)),
            }
            update
        }
        ServerMessage::Static(id, type_id, update, data) => {
            match vals.static_values.get(&id) {
                Some(value) => value.update_value(type_id, &data)?,
                None => return Err(format!("Static with id {} not found", id)),
            }
            update
        }
        ServerMessage::ValueTake(id, type_id, blocking, update, data) => {
            match vals.values_take.get(&id) {
                Some(value) => value.update_take(type_id, &data, blocking)?,
                None => return Err(format!("ValueTake with id {} not found", id)),
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
        ServerMessage::ValueVec(id, type_id, update, list_header, data) => {
            match vals.lists.get(&id) {
                Some(value) => value.update_list(type_id, list_header, &data)?,
                None => return Err(format!("List with id {} not found", id)),
            }
            update
        }
        ServerMessage::ValueMap(id, type_id, update, map_header, data) => {
            match vals.maps.get(&id) {
                Some(value) => value.update_map(type_id, map_header, &data)?,
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
