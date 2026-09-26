use egui_states::{Data, DataMulti, DataMultiTake, DataTake, ValueTake};

pub struct ValueTakeStates {
    pub take_text: ValueTake<String>,
    pub take_empty: ValueTake<()>,
}

pub struct NestedDataStates {
    pub buffer: Data<u16>,
}

pub struct DataStates {
    pub bytes: Data<u8>,
    pub samples: Data<f32>,
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

/// Every supported numeric buffer type, owned exclusively by tests.
#[derive(egui_states::State)]
pub struct NumericStates {
    pub u8_data: Data<u8>,
    pub i8_data: Data<i8>,
    pub u16_data: Data<u16>,
    pub i16_data: Data<i16>,
    pub u32_data: Data<u32>,
    pub i32_data: Data<i32>,
    pub u64_data: Data<u64>,
    pub i64_data: Data<i64>,
    pub f32_data: Data<f32>,
    pub f64_data: Data<f64>,
}
