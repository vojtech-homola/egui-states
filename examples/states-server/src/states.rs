// Ganerated by build.rs, do not edit

use egui_states_pyserver::ServerValuesCreator;

pub(crate) fn create_states(c: &mut ServerValuesCreator) {
    c.add_value::<f32>(0.0);
    c.add_image();
    c.add_graphs::<f32>();
}
