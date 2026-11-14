use bytes::Bytes;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, error, unbounded_channel};

use crate::client::client::ClientCore;
use crate::client::states_creator::ValuesList;
use crate::protocol::data::{DataMultiTakeHeader, MultiDataHeader};
use crate::protocol::image::{ImageHeader, ImageMultiHeader};
use crate::protocol::{
    ClientHeader, FastVec, MAX_MSG_COUNT, MSG_SIZE_THRESHOLD, MessageData, ServerHeader, serialize,
    serialize_to_data,
};

pub(crate) enum ChannelMessage {
    Value(u64, u32, bool, MessageData),
    Signal(u64, u32, MessageData),
    Message(MessageData),
    Ack(u64),
    /// An encoded file transfer message.
    Raw(Vec<u8>),
    /// Panics the send task, to test that its panics are contained.
    #[cfg(test)]
    TestPanic,
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

    // Sends fail only once the connection thread has stopped; the UI thread
    // must not panic because of that.
    pub(crate) fn send(&self, msg: ChannelMessage) {
        let _ = self.sender.send(Some(msg));
    }

    pub(crate) fn send_message(&self, msg: &String) {
        let data = serialize(msg).unwrap();
        self.send(ChannelMessage::Message(data));
    }

    pub(crate) fn send_ack(&self, id: u64) {
        self.send(ChannelMessage::Ack(id));
    }

    pub(crate) fn close(&self) {
        let _ = self.sender.send(None);
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
        ChannelMessage::Raw(message) => data.extend_from_data(&FastVec::<0>::Heap(message)),
        #[cfg(test)]
        ChannelMessage::TestPanic => panic!("injected send task panic"),
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
                let mut counter = 1;
                loop {
                    match self.rx.try_recv() {
                        Ok(Some(msg)) => {
                            counter += 1;
                            parse_to_send(msg, &mut message);
                            if counter >= MAX_MSG_COUNT || message.len() > MSG_SIZE_THRESHOLD {
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
            None | Some(None) => None,
        }
    }

    pub(crate) fn close(self) -> UnboundedReceiver<Option<ChannelMessage>> {
        self.rx
    }
}

/// A message from the server: its header and the payload that followed it.
pub(crate) type ServerMessage = (ServerHeader, Bytes);

pub(crate) struct MessagesParser {
    data: Bytes,
    pointer: usize,
    is_empty: bool,
}

/// The end of a `len`-byte payload that starts at `pointer`, if it lies
/// within a frame of `total` bytes. The sizes come from the network, so the
/// sum must not overflow, which a 32-bit `usize` (wasm32) otherwise allows.
fn checked_end(pointer: usize, len: u32, total: usize) -> Option<usize> {
    let end = pointer.checked_add(usize::try_from(len).ok()?)?;
    (end <= total).then_some(end)
}

impl MessagesParser {
    /// Moves past the next `len` payload bytes of the frame, or fails with
    /// `error` when the frame is shorter.
    fn skip(&mut self, len: u32, error: &'static str) -> Result<(), &'static str> {
        self.pointer = checked_end(self.pointer, len, self.data.len()).ok_or(error)?;
        Ok(())
    }

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
        // A Data header's bytes come first; each part is checked on its own,
        // so their sizes are never added.
        let start = self.pointer;
        self.skip(header.header_len(), header.incomplete())?;
        self.skip(header.payload_len(), header.incomplete())?;
        Ok((header, self.data.slice(start..self.pointer)))
    }
}

/// The binary frames a server sends, apart from how they travel.
pub(crate) trait ServerFrames {
    /// Waits for the next frame of server messages. Frames without messages,
    /// such as pings, are skipped.
    async fn next_frame(&mut self) -> Result<Bytes, &'static str>;
}

/// Reads the server messages of the frames of `S`, one at a time.
pub(crate) struct ServerReader<S> {
    frames: S,
    parser: MessagesParser,
}

impl<S: ServerFrames> ServerReader<S> {
    pub(crate) fn new(frames: S) -> Self {
        Self {
            frames,
            parser: MessagesParser::empty(),
        }
    }

    /// Returns the next message, reading a frame when the current one is
    /// used up. An error ends the connection.
    pub(crate) async fn read(&mut self) -> Result<ServerMessage, &'static str> {
        if let Some(message) = self.parser.next()? {
            return Ok(message);
        }
        let frame = self.frames.next_frame().await?;
        let (parser, message) = MessagesParser::from_bytes(frame)?;
        self.parser = parser;
        Ok(message)
    }
}

pub(crate) async fn handle_message(
    message: ServerMessage,
    vals: &ValuesList,
    client: &ClientCore,
) -> Result<(), String> {
    let (header, payload) = message;
    let update = header.update_flag();
    match header {
        ServerHeader::Update(t) => {
            client.update(t);
            return Ok(());
        }
        // The engine never waits here: sinks that block have a worker.
        ServerHeader::Transfer(endpoint, frame, _) => {
            vals.transfers.handle_frame(endpoint, frame, payload);
            return Ok(());
        }
        ServerHeader::Value(id, type_id, ..) => match vals.value(id) {
            Some(value) => value.update_value(type_id, &payload)?,
            None => {
                client.send_ack(id);
                return Err(format!("Value with id {} not found", id));
            }
        },
        ServerHeader::Static(id, type_id, ..) => match vals.static_value(id) {
            Some(value) => value.update_value(type_id, &payload)?,
            None => return Err(format!("Static with id {} not found", id)),
        },
        ServerHeader::ValueTake(id, type_id, blocking, ..) => match vals.value_take(id) {
            Some(value) => value.update_take(type_id, &payload, blocking)?,
            None => {
                if blocking {
                    client.send_ack(id);
                }
                return Err(format!("ValueTake with id {} not found", id));
            }
        },
        ServerHeader::Image(id, header, _) => match vals.image(id) {
            Some(image) => match header {
                ImageHeader::Set(set, image_type) => image.set_image(set, image_type, &payload)?,
                ImageHeader::Update(rect, image_type, _) => {
                    image.update_image(rect, image_type, &payload)?
                }
                ImageHeader::Fill(size, rgba, _) => image.fill_image(size, rgba, &payload)?,
            },
            None => {
                if header.requires_ack() {
                    client.send_ack(id);
                }
                return Err(format!("Image with id {} not found", id));
            }
        },
        ServerHeader::ValueVec(id, type_id, _, list_header, _) => match vals.vec(id) {
            Some(value) => value.update_list(type_id, list_header, &payload)?,
            None => return Err(format!("List with id {} not found", id)),
        },
        ServerHeader::ValueMap(id, type_id, _, map_header, _) => match vals.map(id) {
            Some(value) => value.update_map(type_id, map_header, &payload)?,
            None => return Err(format!("Map with id {} not found", id)),
        },
        ServerHeader::Data(id, header) => match vals.data(id) {
            Some(data) => data.update_data((header, payload))?,
            None => {
                if header.requires_ack() {
                    client.send_ack(id);
                }
                return Err(format!("Data with id {} not found", id));
            }
        },
        ServerHeader::DataTake(id, header, blocking) => match vals.data_take(id) {
            Some(data_take) => data_take.update((header, payload), blocking)?,
            None => {
                if blocking && header.is_terminal() {
                    client.send_ack(id);
                }
                return Err(format!("DataTake with id {} not found", id));
            }
        },
        ServerHeader::MultiData(id, header) => match vals.data_multi(id) {
            Some(multi_data) => match header {
                MultiDataHeader::Remove(key, _) => multi_data.remove(key),
                MultiDataHeader::Reset(_) => multi_data.reset(),
                MultiDataHeader::Modify(key, header) => {
                    multi_data.update(key, (header, payload))?
                }
            },
            None => {
                if header.requires_ack() {
                    client.send_ack(id);
                }
                return Err(format!("MultiData with id {} not found", id));
            }
        },
        ServerHeader::DataMultiTake(id, header) => match vals.data_multi_take(id) {
            Some(data_multi_take) => match header {
                DataMultiTakeHeader::Remove(key, _) => data_multi_take.remove(key),
                DataMultiTakeHeader::Reset(_) => data_multi_take.reset(),
                DataMultiTakeHeader::Modify(key, header, blocking) => {
                    data_multi_take.update(key, (header, payload), blocking)?
                }
            },
            None => {
                if header.requires_ack_on_failure() {
                    client.send_ack(id);
                }
                return Err(format!("DataMultiTake with id {} not found", id));
            }
        },
        ServerHeader::ImageMulti(id, header) => match vals.image_multi(id) {
            Some(images) => match header {
                ImageMultiHeader::Remove(index, _) => images.remove(index),
                ImageMultiHeader::Reset(_) => images.reset(),
                ImageMultiHeader::Modify(index, ImageHeader::Set(set, image_type), _) => {
                    images.set_image(index, set, image_type, &payload)?
                }
                ImageMultiHeader::Modify(index, ImageHeader::Update(rect, image_type, _), _) => {
                    images.update_image(index, rect, image_type, &payload)?
                }
                ImageMultiHeader::Modify(index, ImageHeader::Fill(size, rgba, _), _) => {
                    images.fill_image(index, size, rgba, &payload)?
                }
            },
            None => {
                if header.requires_ack() {
                    client.send_ack(id);
                }
                return Err(format!("ImageMulti with id {} not found", id));
            }
        },
    }

    if update {
        client.update(0.);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::client::ClientCore;
    use crate::client::states_creator::StatesCreatorClient;
    use crate::protocol::collections::VecHeader;
    use crate::protocol::data::{DataHeader, DataTakeHeader};
    use crate::protocol::data::{DataType, TransportType};
    use crate::protocol::image::ImageSetHeader;
    use crate::protocol::image::ImageType;
    use tokio::sync::mpsc::{UnboundedReceiver, error::TryRecvError};

    /// Frames that were queued in advance.
    struct QueuedFrames(std::collections::VecDeque<Bytes>);

    impl ServerFrames for QueuedFrames {
        async fn next_frame(&mut self) -> Result<Bytes, &'static str> {
            self.0.pop_front().ok_or("Connection closed by server")
        }
    }

    #[test]
    fn server_reader_reads_every_message_of_every_frame() {
        let update = |dt| {
            crate::protocol::serialize::<_, 32>(&ServerHeader::Update(dt))
                .unwrap()
                .into_bytes()
        };
        let both: Vec<u8> = [update(1.0), update(2.0)].concat();
        let frames = QueuedFrames([Bytes::from(both), update(3.0)].into());
        let mut reader = ServerReader::new(frames);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            for expected in [1.0, 2.0, 3.0] {
                let (header, payload) = reader.read().await.unwrap();
                assert!(matches!(header, ServerHeader::Update(dt) if dt == expected));
                assert!(payload.is_empty());
            }
            assert!(matches!(
                reader.read().await,
                Err("Connection closed by server")
            ));
        });
    }

    /// Payload sizes come from the network, so their end is computed without
    /// overflow, also for a pointer near the top of the address space (D7).
    #[test]
    fn payload_bounds_do_not_overflow() {
        assert_eq!(checked_end(2, 3, 5), Some(5));
        assert_eq!(checked_end(2, 4, 5), None);
        assert_eq!(checked_end(0, u32::MAX, 10), None);
        assert_eq!(checked_end(usize::MAX - 1, 2, usize::MAX), None);
        assert_eq!(checked_end(usize::MAX, u32::MAX, usize::MAX), None);
    }

    /// A frame that declares a payload far beyond its own length is a parse
    /// error, not a panic.
    #[test]
    fn oversized_payload_sizes_are_parse_errors() {
        let headers = [
            ServerHeader::Value(1, 0, false, u32::MAX),
            ServerHeader::ValueVec(1, 0, false, VecHeader::Add, u32::MAX),
            ServerHeader::Image(1, ImageHeader::Fill([1, 1], [0; 4], false), u32::MAX),
            ServerHeader::Data(
                1,
                DataHeader::All(
                    DataType::U8,
                    TransportType::Set(1),
                    false,
                    u32::MAX,
                    crate::protocol::data::HeaderUpdate::Keep,
                ),
            ),
            ServerHeader::MultiData(1, MultiDataHeader::Modify(2, DataHeader::Batch(u32::MAX))),
            ServerHeader::DataTake(1, DataTakeHeader::Batch(u32::MAX), false),
        ];
        for header in headers {
            let bytes = postcard::to_stdvec(&header).unwrap();
            assert!(MessagesParser::from_bytes(Bytes::from(bytes)).is_err());
        }
    }

    fn dispatch_missing(message: ServerMessage) -> UnboundedReceiver<Option<ChannelMessage>> {
        let (sender, receiver) = MessageSender::new();
        let context = egui::Context::default();
        let creator = StatesCreatorClient::new(sender.clone(), context.clone(), "root".to_string());
        let values = creator.get_values();
        let client = ClientCore::new(context, sender);
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

    /// UI handles keep sending after the connection thread is gone.
    #[test]
    fn sending_after_the_receiver_is_dropped_does_not_panic() {
        let (sender, receiver) = MessageSender::new();
        drop(receiver);
        sender.send_ack(1);
        sender.send_message(&"late".to_string());
        sender.close();
    }

    #[test]
    fn missing_value_and_blocking_value_take_ack() {
        assert_ack(
            dispatch_missing((ServerHeader::Value(71, 0, false, 0), Bytes::new())),
            71,
        );
        assert_ack(
            dispatch_missing((ServerHeader::ValueTake(72, 0, true, false, 0), Bytes::new())),
            72,
        );
        assert_no_ack(dispatch_missing((
            ServerHeader::ValueTake(73, 0, false, false, 0),
            Bytes::new(),
        )));
    }

    #[test]
    fn missing_image_acks_only_ack_boundaries() {
        assert_ack(
            dispatch_missing((
                ServerHeader::Image(74, ImageHeader::Fill([1, 1], [0; 4], false), 0),
                Bytes::new(),
            )),
            74,
        );
        assert_no_ack(dispatch_missing((
            ServerHeader::Image(
                75,
                ImageHeader::Set(ImageSetHeader::Start([1, 1], 1), ImageType::ColorAlpha),
                0,
            ),
            Bytes::new(),
        )));
    }

    #[test]
    fn missing_image_multi_acks_only_terminal_modifications() {
        assert_ack(
            dispatch_missing((
                ServerHeader::ImageMulti(
                    83,
                    ImageMultiHeader::Modify(7, ImageHeader::Fill([1, 1], [0; 4], true), 0),
                ),
                Bytes::new(),
            )),
            83,
        );
        assert_no_ack(dispatch_missing((
            ServerHeader::ImageMulti(
                84,
                ImageMultiHeader::Modify(
                    7,
                    ImageHeader::Set(ImageSetHeader::Start([1, 1], 1), ImageType::ColorAlpha),
                    0,
                ),
            ),
            Bytes::new(),
        )));
        assert_no_ack(dispatch_missing((
            ServerHeader::ImageMulti(85, ImageMultiHeader::Remove(7, true)),
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

        let (mut parser, (header, data)) = MessagesParser::from_bytes(Bytes::from(bytes)).unwrap();
        assert!(header.update_flag());
        match header {
            ServerHeader::ImageMulti(
                90,
                ImageMultiHeader::Modify(
                    7,
                    ImageHeader::Set(ImageSetHeader::All([2, 1], true), ImageType::ColorAlpha),
                    8,
                ),
            ) => assert_eq!(data.as_ref(), &[1, 2, 3, 4, 5, 6, 7, 8]),
            _ => panic!("unexpected ImageMulti set message"),
        }
        let (header, data) = parser.next().unwrap().unwrap();
        assert!(header.update_flag() && data.is_empty());
        assert!(matches!(
            header,
            ServerHeader::ImageMulti(90, ImageMultiHeader::Remove(2, true))
        ));
        let (header, _) = parser.next().unwrap().unwrap();
        assert!(!header.update_flag());
        assert!(matches!(
            header,
            ServerHeader::ImageMulti(90, ImageMultiHeader::Reset(false))
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
            dispatch_missing((
                ServerHeader::Data(
                    76,
                    DataHeader::All(
                        DataType::U8,
                        TransportType::Set(1),
                        false,
                        0,
                        crate::protocol::data::HeaderUpdate::Keep,
                    ),
                ),
                Bytes::from_static(&[1]),
            )),
            76,
        );
        assert_no_ack(dispatch_missing((
            ServerHeader::Data(77, DataHeader::StartBatch(1, 0)),
            Bytes::from_static(&[1]),
        )));
        assert_ack(
            dispatch_missing((
                ServerHeader::MultiData(
                    78,
                    MultiDataHeader::Modify(
                        3,
                        DataHeader::All(
                            DataType::U8,
                            TransportType::Set(1),
                            false,
                            0,
                            crate::protocol::data::HeaderUpdate::Keep,
                        ),
                    ),
                ),
                Bytes::from_static(&[1]),
            )),
            78,
        );
    }

    #[test]
    fn missing_take_data_acks_only_blocking_terminal_operations() {
        assert_ack(
            dispatch_missing((
                ServerHeader::DataTake(
                    79,
                    DataTakeHeader::All(DataType::U8, 1, false, 0, None),
                    true,
                ),
                Bytes::from_static(&[1]),
            )),
            79,
        );
        assert_no_ack(dispatch_missing((
            ServerHeader::DataTake(80, DataTakeHeader::StartBatch(1, 0), true),
            Bytes::from_static(&[1]),
        )));
        assert_ack(
            dispatch_missing((
                ServerHeader::DataMultiTake(
                    81,
                    DataMultiTakeHeader::Modify(
                        3,
                        DataTakeHeader::All(DataType::U8, 1, false, 0, None),
                        true,
                    ),
                ),
                Bytes::from_static(&[1]),
            )),
            81,
        );
        assert_no_ack(dispatch_missing((
            ServerHeader::DataMultiTake(
                82,
                DataMultiTakeHeader::Modify(
                    3,
                    DataTakeHeader::All(DataType::U8, 1, false, 0, None),
                    false,
                ),
            ),
            Bytes::from_static(&[1]),
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
            matches!(first, (ServerHeader::Value(17, 2, false, 2), ref data) if data.as_ref() == [1, 2])
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
        let context = egui::Context::default();
        let mut creator = StatesCreatorClient::new(sender.clone(), context.clone(), "root".into());
        let value: crate::Value<i32> = creator.value("number", 37);
        let values = creator.get_values();
        let id = *values.states.keys().next().unwrap();
        let type_id = <i32 as Typed>::get_type().get_hash();
        let client = ClientCore::new(context, sender);
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
                        (ServerHeader::Value(id, kind, false, 0), payload),
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
                (
                    ServerHeader::Value(id, type_id, false, 0),
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

#[cfg(test)]
mod serializer_tests {
    use super::*;
    use crate::protocol::{VALUE_MAX_SIZE, WEBSOCKET_MAX_SIZE};

    /// Splits one outgoing frame into its client messages.
    fn headers(frame: &FastVec<64>) -> Vec<ClientHeader> {
        let bytes = match frame {
            FastVec::Heap(data) => data.clone(),
            FastVec::Stack(data) => data.as_ref().to_vec(),
        };
        let mut rest = &bytes[..];
        let mut headers = Vec::new();
        while !rest.is_empty() {
            let (header, after) = postcard::take_from_bytes::<ClientHeader>(rest).unwrap();
            let payload = match header {
                ClientHeader::Value(_, _, _, size)
                | ClientHeader::Signal(_, _, size)
                | ClientHeader::Message(size)
                | ClientHeader::Transfer(_, _, size) => size as usize,
                ClientHeader::Ack(_) | ClientHeader::Handshake(..) => 0,
            };
            rest = &after[payload..];
            headers.push(header);
        }
        headers
    }

    fn frames(sender: &MessageSender, serializer: &mut MessagesSerializer) -> Vec<FastVec<64>> {
        sender.close();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut frames = Vec::new();
        while let Some(frame) = runtime.block_on(serializer.next()) {
            frames.push(frame);
        }
        frames
    }

    /// Same batching contract as the server: at most `MAX_MSG_COUNT` messages.
    #[test]
    fn batches_hold_at_most_max_msg_count_messages_in_order() {
        let (sender, receiver) = MessageSender::new();
        let mut serializer = MessagesSerializer::new(receiver);
        let total = MAX_MSG_COUNT as u64 + 5;
        for id in 0..total {
            sender.send_ack(id);
        }
        let frames = frames(&sender, &mut serializer);
        let mut acked = Vec::new();
        for frame in &frames {
            let headers = headers(frame);
            assert!(
                headers.len() <= MAX_MSG_COUNT,
                "a frame held {} messages",
                headers.len()
            );
            acked.extend(headers.into_iter().map(|header| match header {
                ClientHeader::Ack(id) => id,
                _ => panic!("unexpected message"),
            }));
        }
        assert_eq!(acked, (0..total).collect::<Vec<_>>());
    }

    /// Values may each be as large as `VALUE_MAX_SIZE`; batching them must not
    /// produce a frame the server rejects.
    #[test]
    fn batches_of_maximum_values_stay_within_the_frame_limit() {
        let (sender, receiver) = MessageSender::new();
        let mut serializer = MessagesSerializer::new(receiver);
        let payload = vec![7u8; VALUE_MAX_SIZE];
        for id in 0..15 {
            sender.send(ChannelMessage::Value(
                id,
                1,
                false,
                FastVec::Heap(payload.clone()),
            ));
        }
        let frames = frames(&sender, &mut serializer);
        let mut ids = Vec::new();
        for frame in &frames {
            assert!(
                frame.len() <= WEBSOCKET_MAX_SIZE,
                "frame of {} bytes exceeds the {WEBSOCKET_MAX_SIZE} byte limit",
                frame.len()
            );
            ids.extend(headers(frame).into_iter().map(|header| match header {
                ClientHeader::Value(id, ..) => id,
                _ => panic!("unexpected message"),
            }));
        }
        assert_eq!(ids, (0..15).collect::<Vec<_>>());
    }

    #[test]
    fn close_flushes_queued_messages_and_then_stops() {
        let (sender, receiver) = MessageSender::new();
        let mut serializer = MessagesSerializer::new(receiver);
        sender.send_ack(1);
        sender.send_message(&"diagnostic".to_string());
        let frames = frames(&sender, &mut serializer);
        assert_eq!(frames.len(), 1);
        assert!(matches!(
            headers(&frames[0])[..],
            [ClientHeader::Ack(1), ClientHeader::Message(_)]
        ));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert!(runtime.block_on(serializer.next()).is_none());
    }
}
