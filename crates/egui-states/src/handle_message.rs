use egui_states_core::serialization::{
    ClientHeader, FastVec, ServerHeader, deserialize_from, serialize_to_data,
};

use crate::client_base::Client;
use crate::client_states::ValuesList;
use crate::sender::ChannelMessage;

pub(crate) fn parse_to_send(message: ChannelMessage, data: FastVec<64>) -> FastVec<64> {
    match message {
        ChannelMessage::Value(id, signal, msg_data) => {
            let header = ClientHeader::Value(id, signal, msg_data.len() as u32);
            let mut message = serialize_to_data(&header, data);
            message.extend_from_data(&msg_data);
            message
        }
        ChannelMessage::Signal(id, msg_data) => {
            let header = ClientHeader::Signal(id, msg_data.len() as u32);
            let mut message = serialize_to_data(&header, data);
            message.extend_from_data(&msg_data);
            message
        }
        ChannelMessage::Ack(id) => {
            let header = ClientHeader::Ack(id);
            serialize_to_data(&header, data)
        }
        ChannelMessage::Error(err) => {
            let header = ClientHeader::Error(err);
            serialize_to_data(&header, data)
        }
    }
}

pub(crate) async fn handle_message(
    message_data: &[u8],
    vals: &ValuesList,
    client: &Client,
) -> Result<(), String> {
    let (header, data) = deserialize_from::<ServerHeader>(message_data)?;

    let update = match header {
        ServerHeader::Update(t) => {
            client.update(t);
            return Ok(());
        }
        ServerHeader::Value(id, update) => {
            match vals.values.get(&id) {
                Some(value) => value.update_value(data)?,
                None => return Err(format!("Value with id {} not found", id)),
            }
            update
        }
        ServerHeader::Static(id, update) => {
            match vals.static_values.get(&id) {
                Some(value) => value.update_value(data)?,
                None => return Err(format!("Static with id {} not found", id)),
            }
            update
        }
        ServerHeader::Image(id, update, image_header) => {
            match vals.images.get(&id) {
                Some(value) => value.update_image(image_header, data)?,
                None => return Err(format!("Image with id {} not found", id)),
            }
            update
        }
        ServerHeader::List(id, update, list_header) => {
            match vals.lists.get(&id) {
                Some(value) => value.update_list(list_header, data)?,
                None => return Err(format!("List with id {} not found", id)),
            }
            update
        }
        ServerHeader::Map(id, update, map_header) => {
            match vals.maps.get(&id) {
                Some(value) => value.update_map(map_header, data)?,
                None => return Err(format!("Map with id {} not found", id)),
            }
            update
        }
        ServerHeader::Graph(id, update, header) => {
            match vals.graphs.get(&id) {
                Some(value) => value.update_graph(header, data)?,
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
