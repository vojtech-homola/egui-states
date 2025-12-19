use gui::main_app::MainApp;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let viewport = egui::ViewportBuilder::default().with_inner_size([700., 730.]);

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let port = 8081;

    eframe::run_native(
        "ImagesTest",
        native_options,
        Box::new(|cc| MainApp::new(cc, port)),
    )
    .unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("No window")
            .document()
            .expect("No document");

        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("Failed to find the_canvas_id")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("the_canvas_id was not a HtmlCanvasElement");

        // let location = web_sys::window().unwrap().location();
        // let hostname = location.hostname().unwrap_or_default();
        // let port_str = location.port().unwrap_or_default();
        // let mut port: u16 = if port_str.is_empty() {
        //     8080
        // } else {
        //     port_str.parse().unwrap_or(8080)
        // };
        // port += 1;

        // log::info!("Hostname: {hostname}");
        // log::info!("Starting eframe WebRunner with port {}", port);
        let port = 8081;

        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(move |cc| MainApp::new(cc, port)),
            )
            .await
            .expect("Failed to start eframe WebRunner");
    });
}
