//! Headless smoke scenarios against the actual example setup functions.
use counter_gui::CounterState;
use egui_states::{Client, ClientBuilder, ConnectionState};
use std::time::{Duration, Instant};

pub fn wait(label: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out: {label}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Connection(Client);
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.disconnect();
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.0.get_state() == ConnectionState::Connected && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn connect(client: &Client) {
    wait("connection", || {
        client.connect();
        client.get_state() == ConnectionState::Connected
    });
}

pub fn counter(port: u16) {
    let (state, client) = ClientBuilder::<CounterState>::new().build(port);
    let connection = Connection(client);
    connect(&connection.0);
    // The fixture verifies setup starts at zero, then seeds a non-default value
    // so readiness is established by received data, not the client's default.
    wait("counter initial synchronization", || {
        state.count.get() == -1
    });
    state.count.set_signal(17);
    // The reset's server-to-client response proves the GUI value was synchronized.
    state.reset.set(());
    wait("counter reset", || state.count.get() == 0);
    state.count.set_signal(42);
    // A second reset roundtrip ensures the server has processed edits before exit.
    state.reset.set(());
    wait("second counter reset", || state.count.get() == 0);
}

pub fn showcase(port: u16) {
    let context = egui::Context::default();
    let (state, client) = ClientBuilder::<showcase_gui_core::ShowcaseState>::new()
        .context(context.clone())
        .build(port);
    state.image.image.initialize(
        &context,
        egui::ColorImage::filled([256, 256], egui::Color32::BLACK),
    );
    state.image.images.initialize(&context);
    let connection = Connection(client);
    connect(&connection.0);
    wait("showcase initial count", || state.values.count.get() == 7);
    wait("showcase vector defaults", || {
        state.value_vec.items.read(|v| v == &[10, -3, 27])
    });
    wait("showcase map defaults", || {
        state
            .value_map
            .items
            .read(|v| *v == showcase_server::default_map())
    });
    wait("showcase title", || {
        state.values.title.get() == "Interactive egui-states showcase"
    });
    wait("sample data", || {
        state
            .data
            .samples
            .read(|v| v.len() == 20 * 1024 && v[0] == 0.0 && v[v.len() - 1] == 1.0)
    });
    wait("sparse images", || {
        state.image.images.indices() == vec![2, 7]
    });
    assert_eq!(state.image.images.get_size(7), Some([256, 256]));
    state.value_vec.actions.append_item.set(());
    wait("append", || {
        state.value_vec.items.read(|v| v == &[10, -3, 27, 32])
    });
    state.value_vec.actions.remove_last.set(());
    wait("remove", || {
        state.value_vec.items.read(|v| v == &[10, -3, 27])
    });
    // Serial roundtrips avoid coalescing repeated single-mode signals.
    for length in (0..3).rev() {
        state.value_vec.actions.remove_last.set(());
        wait("empty vector", || {
            state.value_vec.items.read(|v| v.len() == length)
        });
    }
    state.value_vec.actions.append_item.set(());
    wait("append to empty vector", || {
        state.value_vec.items.read(|v| v == &[10])
    });
    state.value_vec.actions.reset_demo.set(());
    wait("reset vector", || {
        state.value_vec.items.read(|v| v == &[10, -3, 27])
    });
    state.value_map.actions.insert_next.set(());
    wait("insert map", || {
        state.value_map.items.read(|v| v.get(&6) == Some(&600))
    });
    state.value_map.actions.remove_lowest.set(());
    wait("remove map", || {
        state
            .value_map
            .items
            .read(|v| !v.contains_key(&1) && v.len() == 3)
    });
    state.value_map.actions.reset_demo.set(());
    wait("reset map", || {
        state
            .value_map
            .items
            .read(|v| *v == showcase_server::default_map())
    });
    state.values.count.set_signal(42);
    // A collection roundtrip gives the receiver a chance to dispatch the edit;
    // server-side smoke assertions independently wait for that callback.
    state.value_vec.actions.append_item.set(());
    wait("final roundtrip", || {
        state.value_vec.items.read(|v| v.len() == 4)
    });
}
