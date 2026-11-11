use egui_states::{Data, DataMulti, DataMultiTake, DataTake, ValueTake};

pub struct ValueTakeStates {
    pub take_text: ValueTake<String>,
    pub take_empty: ValueTake<()>,
}

pub struct NestedDataStates {
    pub buffer: Data<u16>,
}

/// Describes the numbers of a measurement; the server sends it as the header
/// of the buffer, so the GUI always shows a buffer with its own description.
#[egui_states::typed]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measurement {
    pub label: String,
    pub unit: String,
    pub sample_rate: f32,
}

pub struct DataStates {
    pub bytes: Data<u8>,
    pub samples: Data<f32>,
    pub measurement: Data<f32, Measurement>,
    pub nested: NestedDataStates,
}

pub struct NestedMultiDataStates {
    pub buffer: DataMulti<u16>,
}

pub struct MultiDataStates {
    pub bytes: DataMulti<u8>,
    pub samples: DataMulti<f32>,
    pub nested: NestedMultiDataStates,
}

pub struct DataTakeStates {
    pub take_buffer: DataTake<u8>,
    pub take_samples: DataTake<f32>,
}

pub struct NestedMultiDataTakeStates {
    pub buffer: DataMultiTake<u16>,
}

pub struct MultiDataTakeStates {
    pub bytes: DataMultiTake<u8>,
    pub samples: DataMultiTake<f32>,
    pub nested: NestedMultiDataTakeStates,
}

impl egui_states::State for ValueTakeStates {
    const NAME: &'static str = "ValueTakeStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            take_text: c.value_take("take_text"),
            take_empty: c.value_take("take_empty"),
        }
    }
}

impl egui_states::State for DataTakeStates {
    const NAME: &'static str = "DataTakeStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            take_buffer: c.data_take("take_buffer"),
            take_samples: c.data_take("take_samples"),
        }
    }
}

impl egui_states::State for NestedMultiDataTakeStates {
    const NAME: &'static str = "NestedMultiDataTakeStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            buffer: c.data_multi_take("buffer"),
        }
    }
}

impl egui_states::State for MultiDataTakeStates {
    const NAME: &'static str = "MultiDataTakeStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            bytes: c.data_multi_take("bytes"),
            samples: c.data_multi_take("samples"),
            nested: c.substate("nested"),
        }
    }
}

impl egui_states::State for NestedDataStates {
    const NAME: &'static str = "NestedDataStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            buffer: c.data("buffer"),
        }
    }
}

impl egui_states::State for DataStates {
    const NAME: &'static str = "DataStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            bytes: c.data("bytes"),
            samples: c.data("samples"),
            measurement: c.data("measurement"),
            nested: c.substate("nested"),
        }
    }
}

impl egui_states::State for NestedMultiDataStates {
    const NAME: &'static str = "NestedMultiDataStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            buffer: c.data_multi("buffer"),
        }
    }
}

impl egui_states::State for MultiDataStates {
    const NAME: &'static str = "MultiDataStates";

    fn new(c: &mut impl egui_states::StatesCreator) -> Self {
        Self {
            bytes: c.data_multi("bytes"),
            samples: c.data_multi("samples"),
            nested: c.substate("nested"),
        }
    }
}
