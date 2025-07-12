use gui::main_app::MainApp;

fn main() {
    let viewport = egui::ViewportBuilder::default().with_inner_size([700., 730.]);

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "ImagesTest",
        native_options,
        Box::new(|cc| MainApp::new(cc)),
    )
    .unwrap();
}
