use bytes::Bytes;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, error, unbounded_channel};

use crate::client::client::Client;
use crate::client::data::{DataMessage, DataMultiMessage};
use crate::client::data_take::{DataMultiTakeMessage, DataTakeMessage};
use crate::client::image::{ImageMessage, ImageMultiMessage, ImageSetMessage};
use crate::client::states_creator::ValuesList;
use crate::collections::{MapHeader, VecHeader};
use crate::data_transport::{DataHeader, DataMultiTakeHeader, DataTakeHeader, MultiDataHeader};
use crate::image_transport::{ImageHeader, ImageMultiHeader, ImageSetHeader};
use crate::serialization::{
    ClientHeader, FastVec, MAX_MSG_COUNT, MSG_SIZE_THRESHOLD, MessageData, ServerHeader, serialize,
    serialize_to_data,
};

pub(crate) enum ChannelMessage {
    Value(u64, u32, bool, MessageData),
    Signal(u64, u32, MessageData),
    Message(MessageData),
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

    pub(crate) fn send_message(&self, msg: &String) {
        let data = serialize(msg).unwrap();
        self.send(ChannelMessage::Message(data));
    }

    pub(crate) fn send_ack(&self, id: u64) {
        self.send(ChannelMessage::Ack(id));
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
        ChannelMessage::Message(msg_data) => {
            let header = ClientHeader::Message(msg_data.len() as u32);
            serialize_to_data(&header, data).unwrap();
            data.extend_from_data(&msg_data);
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
    Image(u64, bool, ImageMessage, Bytes),
    ValueVec(u64, u32, bool, VecHeader, Bytes),
    ValueMap(u64, u32, bool, MapHeader, Bytes),
    Data(u64, bool, DataMessage),
    DataTake(u64, bool, bool, DataTakeMessage),
    DataMulti(u64, bool, DataMultiMessage),
    DataMultiTake(u64, bool, DataMultiTakeMessage),
    ImageMulti(u64, bool, ImageMultiMessage, Bytes),
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
            ServerHeader::Image(id, header, size) => {
                let (message, update, data) = self._process_image(header, size)?;
                ServerMessage::Image(id, update, message, data)
            }
            ServerHeader::Data(id, data_header) => {
                let (data_message, update) = self._process_data(data_header)?;
                ServerMessage::Data(id, update, data_message)
            }
            ServerHeader::DataTake(id, data_take_header, blocking) => {
                let (data_take_message, update) = self._process_data_take(data_take_header)?;
                ServerMessage::DataTake(id, blocking, update, data_take_message)
            }
            ServerHeader::MultiData(id, multi_data_header) => match multi_data_header {
                MultiDataHeader::Remove(key, update) => {
                    ServerMessage::DataMulti(id, update, DataMultiMessage::Remove(key))
                }
                MultiDataHeader::Reset(update) => {
                    ServerMessage::DataMulti(id, update, DataMultiMessage::Reset)
                }
                MultiDataHeader::Modify(key, data_header) => {
                    let (data_message, update) = self._process_data(data_header)?;
                    ServerMessage::DataMulti(
                        id,
                        update,
                        DataMultiMessage::Modify(key, data_message),
                    )
                }
            },
            ServerHeader::DataMultiTake(id, data_multi_take_header) => match data_multi_take_header
            {
                DataMultiTakeHeader::Remove(key, update) => {
                    ServerMessage::DataMultiTake(id, update, DataMultiTakeMessage::Remove(key))
                }
                DataMultiTakeHeader::Reset(update) => {
                    ServerMessage::DataMultiTake(id, update, DataMultiTakeMessage::Reset)
                }
                DataMultiTakeHeader::Modify(key, data_take_header, blocking) => {
                    let (data_take_message, update) = self._process_data_take(data_take_header)?;
                    ServerMessage::DataMultiTake(
                        id,
                        update,
                        DataMultiTakeMessage::Modify(key, data_take_message, blocking),
                    )
                }
            },
            ServerHeader::ImageMulti(id, header) => match header {
                ImageMultiHeader::Remove(index, update) => ServerMessage::ImageMulti(
                    id,
                    update,
                    ImageMultiMessage::Remove(index),
                    Bytes::new(),
                ),
                ImageMultiHeader::Reset(update) => {
                    ServerMessage::ImageMulti(id, update, ImageMultiMessage::Reset, Bytes::new())
                }
                ImageMultiHeader::Modify(index, header, size) => {
                    let (message, update, data) = self._process_image(header, size)?;
                    ServerMessage::ImageMulti(
                        id,
                        update,
                        ImageMultiMessage::Modify(index, message),
                        data,
                    )
                }
            },
        };

        Ok(message_data)
    }

    fn _process_image(
        &mut self,
        header: ImageHeader,
        size: u32,
    ) -> Result<(ImageMessage, bool, Bytes), &'static str> {
        let size = size as usize;
        if self.pointer + size > self.data.len() {
            return Err("Incomplete data for Image message");
        }
        let data = self.data.slice(self.pointer..self.pointer + size);
        self.pointer += size;

        let (message, update) = match header {
            ImageHeader::Set(set_header, image_type) => {
                let mut update = false;
                let set_message = match set_header {
                    ImageSetHeader::All(size, value) => {
                        update = value;
                        ImageSetMessage::All(size)
                    }
                    ImageSetHeader::Start(size, pixels) => ImageSetMessage::Start(size, pixels),
                    ImageSetHeader::Batch(pixels) => ImageSetMessage::Batch(pixels),
                    ImageSetHeader::End(pixels, value) => {
                        update = value;
                        ImageSetMessage::End(pixels)
                    }
                };
                (ImageMessage::Set(set_message, image_type), update)
            }
            ImageHeader::Update(rect, image_type, update) => {
                (ImageMessage::Update(rect, image_type), update)
            }
            ImageHeader::Fill(size, rgba, update) => (ImageMessage::Fill(size, rgba), update),
        };
        Ok((message, update, data))
    }

    fn _process_data(
        &mut self,
        data_header: DataHeader,
    ) -> Result<(DataMessage, bool), &'static str> {
        let res = match data_header {
            DataHeader::All(data_type, transport_type, update, data_size) => {
                let data_size = data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for Data/DataStatic message");
                }

                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                self.pointer += data_size;
                (DataMessage::All(data_type, transport_type, dat), update)
            }
            DataHeader::StartBatch(count, data_size) => {
                let data_size = data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for Data/DataStatic message");
                }

                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                self.pointer += data_size;
                (DataMessage::BatchStart(count, dat), false)
            }
            DataHeader::Batch(data_size) => {
                let data_size = data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for Data/DataStatic message");
                }
                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                self.pointer += data_size;
                (DataMessage::Batch(dat), false)
            }
            DataHeader::End(data_type, transport_type, update, data_size) => {
                let data_size = data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for Data/DataStatic message");
                }
                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                self.pointer += data_size;
                (
                    DataMessage::BatchEnd(data_type, transport_type, dat),
                    update,
                )
            }
            DataHeader::Drain(start, count, update) => (DataMessage::Drain(start, count), update),
            DataHeader::Clear(update) => (DataMessage::Clear, update),
        };
        Ok(res)
    }

    fn _process_data_take(
        &mut self,
        data_header: DataTakeHeader,
    ) -> Result<(DataTakeMessage, bool), &'static str> {
        let res = match data_header {
            DataTakeHeader::All(data_type, count, update, data_size) => {
                let data_size = data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for DataTake message");
                }

                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                self.pointer += data_size;
                (DataTakeMessage::All(data_type, count, dat), update)
            }
            DataTakeHeader::StartBatch(count, data_size) => {
                let data_size = data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for DataTake message");
                }

                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                self.pointer += data_size;
                (DataTakeMessage::BatchStart(count, dat), false)
            }
            DataTakeHeader::Batch(data_size) => {
                let data_size = data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for DataTake message");
                }
                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                self.pointer += data_size;
                (DataTakeMessage::Batch(dat), false)
            }
            DataTakeHeader::End(data_type, count, update, data_size) => {
                let data_size = data_size as usize;
                if self.pointer + data_size > self.data.len() {
                    return Err("Incomplete data for DataTake message");
                }
                let dat = self.data.slice(self.pointer..self.pointer + data_size);
                self.pointer += data_size;
                (DataTakeMessage::BatchEnd(data_type, count, dat), update)
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
                None => {
                    client.send_ack(id);
                    return Err(format!("Value with id {} not found", id));
                }
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
                None => {
                    if blocking {
                        client.send_ack(id);
                    }
                    return Err(format!("ValueTake with id {} not found", id));
                }
            }
            update
        }
        ServerMessage::Image(id, update, image_message, data) => {
            match vals.images.get(&id) {
                Some(value) => match image_message {
                    ImageMessage::Set(set_message, image_type) => {
                        value.set_image(set_message, image_type, &data)?;
                    }
                    ImageMessage::Update(size, image_type) => {
                        value.update_image(size, image_type, &data)?
                    }
                    ImageMessage::Fill(size, rgba) => value.fill_image(size, rgba, &data)?,
                },
                None => {
                    if image_message.requires_ack() {
                        client.send_ack(id);
                    }
                    return Err(format!("Image with id {} not found", id));
                }
            }
            update
        }
        ServerMessage::ValueVec(id, type_id, update, list_header, data) => {
            match vals.vecs.get(&id) {
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
            match vals.data.get(&id) {
                Some(data) => data.update_data(message)?,
                None => {
                    if message.requires_ack() {
                        client.send_ack(id);
                    }
                    return Err(format!("Data with id {} not found", id));
                }
            }
            update
        }
        ServerMessage::DataTake(id, blocking, update, message) => {
            match vals.data_take.get(&id) {
                Some(data_take) => data_take.update(message, blocking)?,
                None => {
                    if blocking && message.is_terminal() {
                        client.send_ack(id);
                    }
                    return Err(format!("DataTake with id {} not found", id));
                }
            }
            update
        }
        ServerMessage::DataMulti(id, update, message) => {
            match vals.multi_data.get(&id) {
                Some(multi_data) => match message {
                    DataMultiMessage::Remove(key) => multi_data.remove(key),
                    DataMultiMessage::Reset => multi_data.reset(),
                    DataMultiMessage::Modify(key, data_message) => {
                        multi_data.update(key, data_message)?
                    }
                },
                None => {
                    if message.requires_ack() {
                        client.send_ack(id);
                    }
                    return Err(format!("MultiData with id {} not found", id));
                }
            }
            update
        }
        ServerMessage::DataMultiTake(id, update, message) => {
            match vals.data_multi_take.get(&id) {
                Some(data_multi_take) => match message {
                    DataMultiTakeMessage::Remove(key) => data_multi_take.remove(key),
                    DataMultiTakeMessage::Reset => data_multi_take.reset(),
                    DataMultiTakeMessage::Modify(key, data_take_message, blocking) => {
                        data_multi_take.update(key, data_take_message, blocking)?
                    }
                },
                None => {
                    if message.requires_ack_on_failure() {
                        client.send_ack(id);
                    }
                    return Err(format!("DataMultiTake with id {} not found", id));
                }
            }
            update
        }
        ServerMessage::ImageMulti(id, update, message, data) => {
            match vals.image_multi.get(&id) {
                Some(images) => match message {
                    ImageMultiMessage::Remove(index) => images.remove(index),
                    ImageMultiMessage::Reset => images.reset(),
                    ImageMultiMessage::Modify(index, image_message) => match image_message {
                        ImageMessage::Set(set_message, image_type) => {
                            images.set_image(index, set_message, image_type, &data)?
                        }
                        ImageMessage::Update(rect, image_type) => {
                            images.update_image(index, rect, image_type, &data)?
                        }
                        ImageMessage::Fill(size, rgba) => {
                            images.fill_image(index, size, rgba, &data)?
                        }
                    },
                },
                None => {
                    if message.requires_ack() {
                        client.send_ack(id);
                    }
                    return Err(format!("ImageMulti with id {} not found", id));
                }
            }
            update
        }
    };

    if update {
        client.update(0.);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::client::Client;
    use crate::client::states_creator::StatesCreatorClient;
    use crate::data_transport::{DataType, TransportType};
    use crate::image_transport::ImageType;
    use tokio::sync::mpsc::{UnboundedReceiver, error::TryRecvError};

    fn dispatch_missing(message: ServerMessage) -> UnboundedReceiver<Option<ChannelMessage>> {
        let (sender, receiver) = MessageSender::new();
        let creator = StatesCreatorClient::new(sender.clone(), "root".to_string());
        let values = creator.get_values();
        let client = Client::new(None, sender);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

        assert!(
            runtime
                .block_on(handle_message(message, &values, &client))
                .is_err()
        );
        receiver
    }

    fn assert_ack(mut receiver: UnboundedReceiver<Option<ChannelMessage>>, expected_id: u64) {
        match receiver.try_recv() {
            Ok(Some(ChannelMessage::Ack(id))) => assert_eq!(id, expected_id),
            _ => panic!("expected an ACK for {expected_id}"),
        }
        assert!(matches!(
            receiver.try_recv(),
            Err(TryRecvError::Empty | TryRecvError::Disconnected)
        ));
    }

    fn assert_no_ack(mut receiver: UnboundedReceiver<Option<ChannelMessage>>) {
        assert!(matches!(
            receiver.try_recv(),
            Err(TryRecvError::Empty | TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn missing_value_and_blocking_value_take_ack() {
        assert_ack(
            dispatch_missing(ServerMessage::Value(71, 0, false, Bytes::new())),
            71,
        );
        assert_ack(
            dispatch_missing(ServerMessage::ValueTake(72, 0, true, false, Bytes::new())),
            72,
        );
        assert_no_ack(dispatch_missing(ServerMessage::ValueTake(
            73,
            0,
            false,
            false,
            Bytes::new(),
        )));
    }

    #[test]
    fn missing_image_acks_only_ack_boundaries() {
        assert_ack(
            dispatch_missing(ServerMessage::Image(
                74,
                false,
                ImageMessage::Fill([1, 1], [0; 4]),
                Bytes::new(),
            )),
            74,
        );
        assert_no_ack(dispatch_missing(ServerMessage::Image(
            75,
            false,
            ImageMessage::Set(ImageSetMessage::Start([1, 1], 1), ImageType::ColorAlpha),
            Bytes::new(),
        )));
    }

    #[test]
    fn missing_image_multi_acks_only_terminal_modifications() {
        assert_ack(
            dispatch_missing(ServerMessage::ImageMulti(
                83,
                true,
                ImageMultiMessage::Modify(7, ImageMessage::Fill([1, 1], [0; 4])),
                Bytes::new(),
            )),
            83,
        );
        assert_no_ack(dispatch_missing(ServerMessage::ImageMulti(
            84,
            false,
            ImageMultiMessage::Modify(
                7,
                ImageMessage::Set(ImageSetMessage::Start([1, 1], 1), ImageType::ColorAlpha),
            ),
            Bytes::new(),
        )));
        assert_no_ack(dispatch_missing(ServerMessage::ImageMulti(
            85,
            true,
            ImageMultiMessage::Remove(7),
            Bytes::new(),
        )));
    }

    fn append_server_message(bytes: &mut Vec<u8>, header: &ServerHeader, payload: &[u8]) {
        bytes.extend(postcard::to_stdvec(header).unwrap());
        bytes.extend(payload);
    }

    #[test]
    fn image_multi_parser_preserves_keys_payloads_and_repaint_flags() {
        let mut bytes = Vec::new();
        append_server_message(
            &mut bytes,
            &ServerHeader::ImageMulti(
                90,
                ImageMultiHeader::Modify(
                    7,
                    ImageHeader::Set(ImageSetHeader::All([2, 1], true), ImageType::ColorAlpha),
                    8,
                ),
            ),
            &[1, 2, 3, 4, 5, 6, 7, 8],
        );
        append_server_message(
            &mut bytes,
            &ServerHeader::ImageMulti(90, ImageMultiHeader::Remove(2, true)),
            &[],
        );
        append_server_message(
            &mut bytes,
            &ServerHeader::ImageMulti(90, ImageMultiHeader::Reset(false)),
            &[],
        );

        let (mut parser, first) = MessagesParser::from_bytes(Bytes::from(bytes)).unwrap();
        match first {
            ServerMessage::ImageMulti(
                90,
                true,
                ImageMultiMessage::Modify(
                    7,
                    ImageMessage::Set(ImageSetMessage::All([2, 1]), ImageType::ColorAlpha),
                ),
                data,
            ) => assert_eq!(data.as_ref(), &[1, 2, 3, 4, 5, 6, 7, 8]),
            _ => panic!("unexpected ImageMulti set message"),
        }
        assert!(matches!(
            parser.next().unwrap(),
            Some(ServerMessage::ImageMulti(
                90,
                true,
                ImageMultiMessage::Remove(2),
                _
            ))
        ));
        assert!(matches!(
            parser.next().unwrap(),
            Some(ServerMessage::ImageMulti(
                90,
                false,
                ImageMultiMessage::Reset,
                _
            ))
        ));
        assert!(parser.next().unwrap().is_none());
    }

    #[test]
    fn image_multi_parser_rejects_truncated_payloads() {
        let header = ServerHeader::ImageMulti(
            91,
            ImageMultiHeader::Modify(
                3,
                ImageHeader::Update([0, 0, 1, 1], ImageType::GrayAlpha, true),
                2,
            ),
        );
        let mut bytes = postcard::to_stdvec(&header).unwrap();
        bytes.push(7);
        assert!(matches!(
            MessagesParser::from_bytes(Bytes::from(bytes)),
            Err("Incomplete data for Image message")
        ));
    }

    #[test]
    fn missing_data_acks_only_terminal_operations() {
        assert_ack(
            dispatch_missing(ServerMessage::Data(
                76,
                false,
                DataMessage::All(
                    DataType::U8,
                    TransportType::Set(1),
                    Bytes::from_static(&[1]),
                ),
            )),
            76,
        );
        assert_no_ack(dispatch_missing(ServerMessage::Data(
            77,
            false,
            DataMessage::BatchStart(1, Bytes::from_static(&[1])),
        )));
        assert_ack(
            dispatch_missing(ServerMessage::DataMulti(
                78,
                false,
                DataMultiMessage::Modify(
                    3,
                    DataMessage::All(
                        DataType::U8,
                        TransportType::Set(1),
                        Bytes::from_static(&[1]),
                    ),
                ),
            )),
            78,
        );
    }

    #[test]
    fn missing_take_data_acks_only_blocking_terminal_operations() {
        assert_ack(
            dispatch_missing(ServerMessage::DataTake(
                79,
                true,
                false,
                DataTakeMessage::All(DataType::U8, 1, Bytes::from_static(&[1])),
            )),
            79,
        );
        assert_no_ack(dispatch_missing(ServerMessage::DataTake(
            80,
            true,
            false,
            DataTakeMessage::BatchStart(1, Bytes::from_static(&[1])),
        )));
        assert_ack(
            dispatch_missing(ServerMessage::DataMultiTake(
                81,
                false,
                DataMultiTakeMessage::Modify(
                    3,
                    DataTakeMessage::All(DataType::U8, 1, Bytes::from_static(&[1])),
                    true,
                ),
            )),
            81,
        );
        assert_no_ack(dispatch_missing(ServerMessage::DataMultiTake(
            82,
            false,
            DataMultiTakeMessage::Modify(
                3,
                DataTakeMessage::All(DataType::U8, 1, Bytes::from_static(&[1])),
                false,
            ),
        )));
    }
}

#[cfg(test)]
mod malformed_tests {
    use super::*;
    use crate::client::states_creator::StatesCreatorClient;
    use crate::{StatesCreator, Typed};

    #[test]
    fn truncated_headers_payloads_and_later_messages_do_not_hide_valid_updates() {
        let header = ServerHeader::Value(17, 2, false, 2);
        let mut full = postcard::to_stdvec(&header).unwrap();
        full.extend([1, 2]);
        for length in 0..full.len() {
            assert!(
                MessagesParser::from_bytes(Bytes::copy_from_slice(&full[..length])).is_err(),
                "accepted truncated frame length {length}"
            );
        }
        let mut mixed = full.clone();
        mixed.push(255);
        let (mut parser, first) = MessagesParser::from_bytes(mixed.into()).unwrap();
        assert!(
            matches!(first, ServerMessage::Value(17, 2, false, ref data) if data.as_ref() == [1, 2])
        );
        assert!(parser.next().is_err());
        assert!(
            MessagesParser::from_bytes(full.into()).is_ok(),
            "next frame remains parseable"
        );
    }

    #[test]
    fn invalid_value_type_and_payload_ack_preserve_state_and_allow_recovery() {
        let (sender, mut receiver) = MessageSender::new();
        let mut creator = StatesCreatorClient::new(sender.clone(), "root".into());
        let value: crate::Value<i32> = creator.value("number", 37);
        let values = creator.get_values();
        let id = *values.values.keys().next().unwrap();
        let type_id = <i32 as Typed>::get_type().get_hash();
        let client = Client::new(None, sender);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        for (kind, payload) in [
            (type_id.wrapping_add(1), Bytes::from_static(&[0])),
            (type_id, Bytes::from_static(&[255])),
        ] {
            assert!(
                runtime
                    .block_on(handle_message(
                        ServerMessage::Value(id, kind, false, payload),
                        &values,
                        &client
                    ))
                    .is_err()
            );
            assert_eq!(value.get(), 37);
            crate::test_support::ack(&mut receiver, id);
        }
        runtime
            .block_on(handle_message(
                ServerMessage::Value(
                    id,
                    type_id,
                    false,
                    postcard::to_stdvec(&-91i32).unwrap().into(),
                ),
                &values,
                &client,
            ))
            .unwrap();
        assert_eq!(value.get(), -91);
        crate::test_support::ack(&mut receiver, id);
    }
}
