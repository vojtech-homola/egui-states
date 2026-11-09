use counter_gui::CounterState;
use eframe::egui;
use egui_states::{Client, ClientBuilder, ConnectionState};

struct CounterApp {
    state: CounterState,
    client: Client,
}

impl eframe::App for CounterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Counter");
            let connected = self.client.get_state() == ConnectionState::Connected;
            ui.label(if connected {
                "Connected"
            } else {
                "Disconnected"
            });
            if !connected && ui.button("Connect").clicked() {
                self.client.connect();
            }
            let mut count = self.state.count.get();
            if ui.add(egui::DragValue::new(&mut count)).changed() {
                self.state.count.set_signal(count);
            }
            if ui.button("Reset").clicked() {
                self.state.reset.set(());
            }
        });
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        "egui-states counter",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([300.0, 160.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            let (state, client) =
                ClientBuilder::<CounterState>::new(cc.egui_ctx.clone(), 8092).build();
            Ok(Box::new(CounterApp { state, client }))
        }),
    )
}
