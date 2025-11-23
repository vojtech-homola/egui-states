use egui_states_core::controls::ControlServer;
use egui_states_core::serialization::{ServerHeader, deserialize_from};

use crate::client_base::Client;
use crate::values_creator::ValuesList;

pub(crate) async fn handle_message(
    message_data: &[u8],
    vals: &ValuesList,
    client: &Client,
) -> Result<(), String> {
    let (header, data) = deserialize_from::<ServerHeader>(message_data)?;

    let update = match header {
        ServerHeader::Control(ControlServer::Update(t)) => {
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
        ServerHeader::Control(_) => false,
    };

    // let message_type = data[0];

    // if message_type == serialization::TYPE_CONTROL {
    //     let control = ControlMessage::deserialize(data)?;
    //     match control {
    //         ControlMessage::Update(t) => {
    //             client.update(t);
    //         }
    //         _ => {}
    //     }
    //     return Ok(());
    // }

    // let id = u32::from_le_bytes(data[1..5].try_into().unwrap());
    // let update = match message_type {
    //     serialization::TYPE_VALUE => match vals.values.get(&id) {
    //         Some(value) => value.update_value(&data[5..])?,
    //         None => return Err(format!("Value with id {} not found", id)),
    //     },

    //     serialization::TYPE_STATIC => match vals.static_values.get(&id) {
    //         Some(value) => value.update_value(&data[5..])?,
    //         None => return Err(format!("Static with id {} not found", id)),
    //     },

    //     serialization::TYPE_IMAGE => match vals.images.get(&id) {
    //         Some(value) => value.update_value(&data[5..])?,
    //         None => return Err(format!("Image with id {} not found", id)),
    //     },

    //     serialization::TYPE_DICT => match vals.maps.get(&id) {
    //         Some(value) => value.update_value(&data[5..])?,
    //         None => return Err(format!("Dict with id {} not found", id)),
    //     },

    //     serialization::TYPE_LIST => match vals.lists.get(&id) {
    //         Some(value) => value.update_value(&data[5..])?,
    //         None => return Err(format!("List with id {} not found", id)),
    //     },

    //     serialization::TYPE_GRAPH => match vals.graphs.get(&id) {
    //         Some(value) => value.update_value(&data[5..])?,
    //         None => return Err(format!("Graph with id {} not found", id)),
    //     },

    //     _ => return Err(format!("Unknown message type: {}", message_type)),
    // };

    if update {
        client.update(0.);
    }

    Ok(())
}
