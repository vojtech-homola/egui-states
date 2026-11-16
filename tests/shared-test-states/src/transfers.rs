use egui_states::{Download, Upload};

/// Metadata sent with the offers of the typed endpoints.
#[egui_states::typed]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransferInfo {
    pub run: u32,
    pub label: String,
}

#[derive(egui_states::State)]
pub struct TransferStates {
    pub files: Download,
    pub results: Download<TransferInfo>,
    pub logs: Upload,
    pub inputs: Upload<TransferInfo>,
}
