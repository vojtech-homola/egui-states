use egui_states_test_process as process;
use example_smoke_tests::{free_port, wait};

fn probe(port: u16, example: &str) {
    process::checked_output(
        std::process::Command::new(env!("CARGO_BIN_EXE_example-probe"))
            .arg(port.to_string())
            .arg(example),
        example,
        30,
    );
}

#[test]
fn showcase_setup_and_client() {
    let demo = showcase_server::setup_server().unwrap();
    let states = &demo.server.states;
    assert_eq!(states.value_vec.items.get().unwrap(), [10, -3, 27]);
    assert_eq!(
        states.value_map.items.get().unwrap(),
        showcase_server::default_map()
    );
    let port = free_port();
    demo.server
        .start(port, Some(std::net::Ipv4Addr::LOCALHOST), None)
        .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let _callback = states.values.count.connect(move |value| {
        tx.send(value).unwrap();
    });
    probe(port, "showcase");
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
        42
    );
    assert_eq!(states.values.count.get().unwrap(), 42);
    demo.server.stop();
    wait("showcase shutdown", || !demo.server.is_running());
}

#[test]
fn counter_setup_and_client() {
    let demo = counter_server::setup_server().unwrap();
    assert_eq!(demo.server.states.count.get().unwrap(), 0);
    demo.server.states.count.set(-1, false).unwrap();
    let port = free_port();
    demo.server
        .start(port, Some(std::net::Ipv4Addr::LOCALHOST), None)
        .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let _callback = demo.server.states.count.connect(move |value| {
        tx.send(value).unwrap();
    });
    probe(port, "counter");
    let first = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    let second = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!([first, second], [17, 42]);
    assert_eq!(demo.server.states.count.get().unwrap(), 0);
    demo.server.stop();
    wait("counter shutdown", || !demo.server.is_running());
}
