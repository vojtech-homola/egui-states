use egui_states::{TransferHandle, TransferStatus};

use super::ViewState;

/// Takes the server's offers; called every frame, so an offer is answered
/// while this section is collapsed, too.
pub(super) fn poll_offers(state: &mut ViewState) {
    let Some(offer) = state.schema.transfers.download.take_offer() else {
        return;
    };
    state.last_offer_note = offer.meta().note.clone();
    // A native client saves into a folder of its own; a browser hands the
    // file to its download manager.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = std::env::temp_dir().join("egui-states-showcase");
        state.download_dir = Some(dir.display().to_string());
        offer.accept_dir(dir);
    }
    #[cfg(target_arch = "wasm32")]
    offer.accept_download();
}

pub(super) fn show_transfers(ui: &mut egui::Ui, state: &mut ViewState) {
    ui.collapsing("transfers", |ui| {
        let t = &state.schema.transfers;
        ui.label("Download<SampleInfo>: root.transfers.download");
        ui.horizontal(|ui| {
            for mebibytes in [1_u32, 100, 1000] {
                if ui.button(format!("download {mebibytes} MiB")).clicked() {
                    t.request_download.set(mebibytes);
                }
            }
        });
        if let Some(dir) = &state.download_dir {
            ui.label(format!("saved into {dir}"));
        }
        if !state.last_offer_note.is_empty() {
            ui.label(format!("offer note: {}", state.last_offer_note));
        }
        show_handle(ui, t.download.last());

        ui.separator();

        ui.label("Upload: root.transfers.upload");
        if ui
            .add_enabled(!t.upload.is_busy(), egui::Button::new("upload 8 MiB"))
            .clicked()
        {
            let data = (0..8 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
            t.upload.send_bytes(data, "upload.bin", ());
        }
        show_handle(ui, t.upload.last());
        ui.label(format!("server: {}", t.upload_result.get()));
    });
}

fn show_handle(ui: &mut egui::Ui, handle: Option<TransferHandle>) {
    let Some(handle) = handle else {
        ui.label("no transfer yet");
        return;
    };
    ui.horizontal(|ui| {
        ui.label(handle.name());
        match handle.status() {
            TransferStatus::Pending => {
                ui.label("waiting");
            }
            TransferStatus::Running => {
                let progress = handle.progress();
                ui.add(
                    egui::ProgressBar::new(progress.fraction().unwrap_or(0.0)).show_percentage(),
                );
                if ui.button("cancel").clicked() {
                    handle.cancel();
                }
            }
            TransferStatus::Done => {
                ui.label("done");
            }
            TransferStatus::Failed(error) => {
                ui.label(format!("failed: {error}"));
            }
            _ => {}
        }
    });
}
