use egui_states_core::controls::{ControlClient, ControlServer};
use egui_states_core::nohash::NoHashMap;
use egui_states_core::serialization::{
    ClientHeader, MessageData, ServerHeader, deserialize, deserialize_from,
    serialize_value_to_message,
};

use crate::client_base::Client;
use crate::client_states::ValuesList;

pub(crate) fn check_types(message_data: &[u8], vals: &ValuesList) -> Result<MessageData, ()> {
    match deserialize_from::<ServerHeader>(message_data) {
        Ok((ServerHeader::Control(ControlServer::TypesAsk), data)) => {
            let types = match deserialize::<NoHashMap<u64, u64>>(data) {
                Ok(t) => t,
                Err(_) => {
                    #[cfg(debug_assertions)]
                    println!("Deserialization types ask data failed.");
                    return Err(());
                }
            };

            let mut types_res = Vec::new();
            for (id, t) in types {
                if let Some(state_type) = vals.types.get(&id) {
                    if *state_type == t {
                        types_res.push(id);
                    }
                }
            }
            let header = ClientHeader::Control(ControlClient::TypesAnswer);
            let data = serialize_value_to_message(types_res);
            let message = header.serialize_message(Some(data));
            Ok(message)
        }
        Ok((_, _)) => {
            #[cfg(debug_assertions)]
            println!("Expected TypesAsk message, got different message.");
            Err(())
        }
        Err(_) => {
            #[cfg(debug_assertions)]
            println!("Deserialization types message failed.");
            Err(())
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

    if update {
        client.update(0.);
    }

    Ok(())
}
