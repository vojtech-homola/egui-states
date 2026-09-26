pub mod collections;
pub mod data;
pub mod image;
pub mod integration;
pub mod sections;

use collections::{ValueMapStates, ValueVecStates};
use data::{DataStates, DataTakeStates, MultiDataStates, MultiDataTakeStates, ValueTakeStates};
use image::ImageStates;
use sections::{CustomValueStates, SignalStates, StaticStates, ValueStates};

#[derive(egui_states::State)]
pub struct State {
    pub integration: integration::IntegrationState,
    pub values: ValueStates,
    pub signals: SignalStates,
    pub statics: StaticStates,
    pub value_take: ValueTakeStates,
    pub custom_values: CustomValueStates,
    pub value_vec: ValueVecStates,
    pub value_map: ValueMapStates,
    pub data: DataStates,
    pub numeric: data::NumericStates,
    pub data_take: DataTakeStates,
    pub multi_data: MultiDataStates,
    pub data_multi_take: MultiDataTakeStates,
    pub image: ImageStates,
}
