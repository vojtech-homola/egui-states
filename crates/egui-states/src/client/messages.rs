use bytes::Bytes;
use std::collections::VecDeque;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::client::client::Client;
use crate::client::data::{
    ChannelMessageData, DataMessage, DataMessageAll, DataMessageEnd, DataMessageHead,
};
use crate::client::states_creator::ValuesList;
use crate::collections::{MapHeader, VecHeader};
use crate::data_header::DataHeader;
use crate::graphs::GraphHeader;
use crate::image_header::ImageHeader;
use crate::serialization::{
    ClientHeader, FastVec, MAX_MSG_COUNT, MSG_SIZE_THRESHOLD, MessageData, ServerHeader,
    serialize_to_data,
};

pub(crate) enum ChannelMessage {
    Value(u64, u32, bool, MessageData),
    Signal(u64, u32, MessageData),
    Ack(u64),
}

#[derive(Clone)]
pub(crate) struct MessageSender {
    sender: UnboundedSender<Option<ChannelMessage>>,
}
impl MessageSender {
    pub(crate) fn new() -> (Self, UnboundedReceiver<Option<ChannelMessage>>) {
        let (sender, receiver) = unbounded_channel();
        (Self { sender }, receiver)
    }

    pub(crate) fn send(&self, msg: ChannelMessage) {
        self.sender.send(Some(msg)).unwrap();
    }

    pub(crate) fn close(&self) {
        self.sender.send(None).unwrap();
    }
}

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

pub(crate) struct MessagesSerializer {
    queue: VecDeque<ChannelMessage>,
    temp: VecDeque<FastVec<64>>,
}

impl MessagesSerializer {
    pub(crate) fn new() -> Self {
        Self {
            queue: VecDeque::with_capacity(MAX_MSG_COUNT),
            temp: VecDeque::new(),
        }
    }

    pub(crate) fn push(&mut self, message: ChannelMessage) {
        self.queue.push_back(message);
    }

    pub(crate) fn serialize(&mut self) -> Option<FastVec<64>> {
        if let Some(message) = self.temp.pop_front() {
            return Some(message);
        }

        if self.queue.is_empty() {
            return None;
        }

        let mut actual = FastVec::<64>::new();
        while let Some(msg) = self.queue.pop_front() {
            match msg {
                ChannelMessage::Value(id, type_id, signal, data) => {
                    let header = ClientHeader::Value(id, type_id, signal, data.len() as u32);
                    serialize_to_data(&header, &mut actual).unwrap();
                    actual.extend_from_data(&data);
                }
                ChannelMessage::Signal(id, type_id, data) => {
                    let header = ClientHeader::Signal(id, type_id, data.len() as u32);
                    serialize_to_data(&header, &mut actual).unwrap();
                    actual.extend_from_data(&data);
                }
                ChannelMessage::Ack(id) => {
                    let header = ClientHeader::Ack(id);
                    serialize_to_data(&header, &mut actual).unwrap();
                }
            }

            if actual.len() >= MSG_SIZE_THRESHOLD {
                break;
            }
        }

        Some(actual)
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
    Data(u64, bool, DataMessage),
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
            ServerHeader::ValueMap(id, type_id, update, header, size) => {
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
            ServerHeader::Data(data_header) => self._process_data(data_header)?,
            ServerHeader::DataStatic(data_header) => self._process_data(data_header)?,
        };

        Ok(message_data)
    }

    fn _process_data(&self, data_header: DataHeader) -> Result<ServerMessage, &'static str> {
        let res = match data_header {
            DataHeader::All(header) => {
                let header_size = header.header_size as usize;
                let data_size = header.data_size as usize;
                if self.pointer + header_size + data_size > self.data.len() {
                    return Err("Incomplete data for Data/DataStatic message");
                }

                let msg = DataMessageAll {
                    type_id: header.type_id,
                    is_add: header.is_add,
                    header: self.data.slice(self.pointer..self.pointer + header_size),
                    data: self
                        .data
                        .slice(self.pointer + header_size..self.pointer + header_size + data_size),
                };
                ServerMessage::Data(header.id, header.update, DataMessage::All(msg))
            }
            DataHeader::Head(header) => {
                let header_size = header.header_size as usize;
                let data_size = header.data_size as usize;
                if self.pointer + header_size + data_size > self.data.len() {
                    return Err("Incomplete data for Data/DataStatic message");
                }

                let msg = DataMessageHead {
                    type_id: header.type_id,
                    data_size_all: header.data_size_all,
                    header: self.data.slice(self.pointer..self.pointer + header_size),
                    data: self
                        .data
                        .slice(self.pointer + header_size..self.pointer + header_size + data_size),
                };
                ServerMessage::Data(header.id, false, DataMessage::Head(msg))
            }
            DataHeader::Data(header) => {
                let data_size = header.data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for Data/DataStatic message");
                }
                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                ServerMessage::Data(header.id, false, DataMessage::Data(dat))
            }
            DataHeader::End(header) => {
                let data_size = header.data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for Data/DataStatic message");
                }
                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                ServerMessage::Data(
                    header.id,
                    header.update,
                    DataMessage::End(DataMessageEnd {
                        is_add: header.is_add,
                        data: dat,
                    }),
                )
            }
        };
        Ok(res)
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
        ServerMessage::Data(id, update, message) => {
            match vals.datas.get(&id) {
                Some(value) => value.update_data(message)?,
                None => return Err(format!("Data with id {} not found", id)),
            }
            update
        }
        ServerMessage::DataStatic(id, update, message) => {
            match vals.data_statics.get(&id) {
                Some(value) => value.update_data(message)?,
                None => return Err(format!("DataStatic with id {} not found", id)),
            }
            update
        }
    };

    if update {
        client.update(0.);
    }

    Ok(())
}
