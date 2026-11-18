//! Application code and synchronized state shared by the showcase entry points.
mod app;
pub mod state;

pub use app::MainApp;
/// How the native GUI checks the server when it connects with TLS.
pub use egui_states::Tls;
pub use state::ShowcaseState;
