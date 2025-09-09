use egui_states_core::controls::ControlMessage;
use egui_states_core::serialization;

use crate::client_state::UIState;
use crate::states_creator::ValuesList;

pub(crate) fn handle_message(data: &[u8], vals: &ValuesList, ui_state: &UIState) -> Result<(), String> {
    let message_type = data[0];

    if message_type == serialization::TYPE_CONTROL {
        let control = ControlMessage::deserialize(data)?;
        match control {
            ControlMessage::Update(t) => {
                ui_state.update(t);
            }
            _ => {}
        }
        return Ok(());
    }

    let id = u32::from_le_bytes(data[1..5].try_into().unwrap());
    let update = match message_type {
        serialization::TYPE_VALUE => match vals.values.get(&id) {
            Some(value) => value.update_value(&data[5..])?,
            None => return Err(format!("Value with id {} not found", id)),
        },

        serialization::TYPE_STATIC => match vals.static_values.get(&id) {
            Some(value) => value.update_value(&data[5..])?,
            None => return Err(format!("Static with id {} not found", id)),
        },

        serialization::TYPE_IMAGE => match vals.images.get(&id) {
            Some(value) => value.update_value(&data[5..])?,
            None => return Err(format!("Image with id {} not found", id)),
        },

        serialization::TYPE_DICT => match vals.dicts.get(&id) {
            Some(value) => value.update_value(&data[5..])?,
            None => return Err(format!("Dict with id {} not found", id)),
        },

        serialization::TYPE_LIST => match vals.lists.get(&id) {
            Some(value) => value.update_value(&data[5..])?,
            None => return Err(format!("List with id {} not found", id)),
        },

        serialization::TYPE_GRAPH => match vals.graphs.get(&id) {
            Some(value) => value.update_value(&data[5..])?,
            None => return Err(format!("Graph with id {} not found", id)),
        },

        _ => return Err(format!("Unknown message type: {}", message_type)),
    };

    if update {
        ui_state.update(0.);
    }

    Ok(())
}
