use bytes::Bytes;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, error, unbounded_channel};

use crate::client::client::Client;
use crate::client::data::DataMessage;
use crate::client::states_creator::ValuesList;
use crate::collections::{MapHeader, VecHeader};
use crate::data_transport::DataHeader;
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

fn parse_to_send(message: ChannelMessage, data: &mut FastVec<64>) {
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
    rx: UnboundedReceiver<Option<ChannelMessage>>,
    stopped: bool,
}

impl MessagesSerializer {
    pub(crate) fn new(rx: UnboundedReceiver<Option<ChannelMessage>>) -> Self {
        Self { rx, stopped: false }
    }

    pub(crate) async fn next(&mut self) -> Option<FastVec<64>> {
        if self.stopped {
            return None;
        }

        match self.rx.recv().await {
            Some(Some(msg)) => {
                let mut message = FastVec::<64>::new();
                parse_to_send(msg, &mut message);
                let mut counter = 0;
                loop {
                    match self.rx.try_recv() {
                        Ok(Some(msg)) => {
                            counter += 1;
                            parse_to_send(msg, &mut message);
                            if counter > MAX_MSG_COUNT || message.len() > MSG_SIZE_THRESHOLD {
                                return Some(message);
                            }
                        }
                        Err(error::TryRecvError::Empty) => {
                            return Some(message);
                        }
                        Ok(None) | Err(error::TryRecvError::Disconnected) => {
                            self.stopped = true;
                            return Some(message);
                        }
                    }
                }
            }
            None | Some(None) => {
                return None;
            }
        }
    }

    pub(crate) fn close(self) -> UnboundedReceiver<Option<ChannelMessage>> {
        self.rx
    }
}

pub(crate) enum ServerMessage {
    Value(u64, u32, bool, Bytes),
    ValueTake(u64, u32, bool, bool, Bytes),
    Static(u64, u32, bool, Bytes),
    Image(u64, bool, ImageHeader, Bytes),
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
            ServerHeader::Image(id, update, header, size) => {
                let size = size as usize;
                if self.pointer + size > self.data.len() {
                    return Err("Incomplete data for Image message");
                }
                let data = self.data.slice(self.pointer..self.pointer + size);
                self.pointer += size;
                ServerMessage::Image(id, update, header, data)
            }
            ServerHeader::Data(id, data_header) => self._process_data(id, data_header)?,
        };

        Ok(message_data)
    }

    fn _process_data(
        &mut self,
        id: u64,
        data_header: DataHeader,
    ) -> Result<ServerMessage, &'static str> {
        let res = match data_header {
            DataHeader::All(data_type, transport_type, update, data_size) => {
                let data_size = data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for Data/DataStatic message");
                }

                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                self.pointer += data_size;
                ServerMessage::Data(id, update, DataMessage::All(data_type, transport_type, dat))
            }
            DataHeader::StartBatch(count, data_size) => {
                let data_size = data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for Data/DataStatic message");
                }

                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                self.pointer += data_size;
                ServerMessage::Data(id, false, DataMessage::BatchStart(count, dat))
            }
            DataHeader::Batch(data_size) => {
                let data_size = data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for Data/DataStatic message");
                }
                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                self.pointer += data_size;
                ServerMessage::Data(id, false, DataMessage::Batch(dat))
            }
            DataHeader::End(data_type, transport_type, update, data_size) => {
                let data_size = data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for Data/DataStatic message");
                }
                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                self.pointer += data_size;
                ServerMessage::Data(
                    id,
                    update,
                    DataMessage::BatchEnd(data_type, transport_type, dat),
                )
            }
            DataHeader::Drain(start, count, update) => {
                ServerMessage::Data(id, update, DataMessage::Drain(start, count))
            }
            DataHeader::Clear(update) => ServerMessage::Data(id, update, DataMessage::Clear),
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
        ServerMessage::Data(id, update, message) => {
            match vals.datas.get(&id) {
                Some(value) => value.update_data(message)?,
                None => return Err(format!("Data with id {} not found", id)),
            }
            update
        }
    };

    if update {
        client.update(0.);
    }

    Ok(())
}
