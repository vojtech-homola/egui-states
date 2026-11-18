use showcase_gui_core::MainApp;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    const USAGE: &str =
        "Usage: showcase-gui [--host HOST] [--port PORT] [--tls any|system|FINGERPRINT]";
    let mut args = std::env::args().skip(1);
    let (mut host, mut port, mut tls) = (None, 8091, None);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(USAGE);
        match arg.as_str() {
            "--host" => host = Some(value()?),
            "--port" => port = value()?.parse::<u16>()?,
            // The fingerprint the server printed, `any`, or `system`.
            "--tls" => tls = Some(value()?.parse::<showcase_gui_core::Tls>()?),
            _ => return Err(USAGE.into()),
        }
    }

    let viewport = eframe::egui::ViewportBuilder::default().with_inner_size([700., 730.]);

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "egui-states showcase",
        native_options,
        Box::new(|cc| MainApp::new(cc, port, host, tls)),
    )?;
    Ok(())
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

        // Used only when the page has no host; otherwise the client connects
        // back to the host and port the page was loaded from.
        let port = 8091;

        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(move |cc| MainApp::new(cc, port, None, None)),
            )
            .await
            .expect("Failed to start eframe WebRunner");
    });
}
