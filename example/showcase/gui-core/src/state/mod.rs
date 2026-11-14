pub mod collections;
pub mod data;
pub mod image;
pub mod sections;
pub mod transfers;

use collections::{ValueMapStates, ValueVecStates};
use data::{DataStates, DataTakeStates, MultiDataStates, MultiDataTakeStates, ValueTakeStates};
use image::ImageStates;
use sections::{CustomValueStates, SignalStates, StaticStates, ValueStates};

#[derive(egui_states::State)]
pub struct ShowcaseState {
    pub values: ValueStates,
    pub signals: SignalStates,
    pub statics: StaticStates,
    pub value_take: ValueTakeStates,
    pub custom_values: CustomValueStates,
    pub value_vec: ValueVecStates,
    pub value_map: ValueMapStates,
    pub data: DataStates,
    pub data_take: DataTakeStates,
    pub multi_data: MultiDataStates,
    pub data_multi_take: MultiDataTakeStates,
    pub image: ImageStates,
    pub transfers: transfers::TransferStates,
}
