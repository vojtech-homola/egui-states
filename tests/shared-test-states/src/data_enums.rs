//! Enums with data in every state kind, header and transfer.
use egui_states::{
    Data, DataMulti, DataMultiTake, DataTake, Download, Image, ImageMulti, MapState, Signal,
    Static, Upload, Value, ValueTake, VecState,
};

use crate::sections::{TestEnum, TestStruct2};

/// Every variant shape an enum with data supports.
#[egui_states::typed(rust_derive(Debug, PartialEq))]
#[derive(Clone, Debug, Default, PartialEq, egui_states::InitialValue)]
pub enum TestCommand {
    #[default]
    Stop,
    MoveTo {
        x: f32,
        y: f32,
    },
    SetGain(f32),
    Pair(i32, String),
    Choose(TestEnum),
    Load {
        path: String,
        summary: Option<TestStruct2>,
    },
    Report(TestStatus),
    /// A Python keyword as a variant name.
    None,
    /// A Python keyword as a field name.
    Keyword {
        from: u8,
    },
}

/// An enum with data inside a struct and inside another enum.
#[egui_states::typed(rust_derive(Debug, PartialEq))]
#[derive(Clone, Debug, Default, PartialEq, egui_states::InitialValue)]
pub enum TestStatus {
    #[default]
    Idle,
    Busy {
        progress: f32,
    },
    Failed(String),
}

#[egui_states::typed(rust_derive(Debug, PartialEq))]
#[derive(Clone, Debug, Default, PartialEq, egui_states::InitialValue)]
pub struct TestJob {
    pub name: String,
    pub status: TestStatus,
}

/// Changed by the `data-enums` scenario: the server sets every state on
/// command 38 and checks what the client sent on command 39.
#[derive(egui_states::State)]
pub struct DataEnumStates {
    #[state(init = TestCommand::MoveTo { x: 1.5, y: -2.0 })]
    pub command: Value<TestCommand>,
    #[state(init = TestCommand::SetGain(0.5))]
    pub fixed: Static<TestCommand>,
    #[state(init = TestJob { name: "job".into(), status: TestStatus::Busy { progress: 0.25 } })]
    pub job: Value<TestJob>,
    #[state(init = Some(TestStatus::Failed("init".into())))]
    pub optional: Value<Option<TestStatus>>,
    #[state(queue)]
    pub commands: Signal<TestCommand>,
    pub take: ValueTake<TestCommand>,
    pub list: VecState<TestCommand>,
    pub table: MapState<u32, TestStatus>,
    pub data: Data<u8, TestCommand>,
    pub data_take: DataTake<u8, TestCommand>,
    pub data_multi: DataMulti<u8, TestCommand>,
    pub data_multi_take: DataMultiTake<u8, TestCommand>,
    pub image: Image<TestCommand>,
    pub images: ImageMulti<TestCommand>,
    pub upload: Upload<TestCommand>,
    pub download: Download<TestCommand>,
}
