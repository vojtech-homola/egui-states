use egui_states_test_process as process;
use egui_states_test_support::{fixture::Fixture, free_port};

fn scenario(scenario: &str) {
    let port = free_port();
    let fixture = Fixture::new(port).unwrap();
    process::checked_output(
        std::process::Command::new(env!("CARGO_BIN_EXE_client-probe"))
            .arg(port.to_string())
            .arg(scenario),
        scenario,
        30,
    );
    fixture.server.stop();
    fixture.assert_no_errors();
}

macro_rules! scenarios {
    ($($name:ident => $scenario:literal),* $(,)?) => {$(
        #[test]
        fn $name() { scenario($scenario); }
    )*};
}

egui_states_test_support::for_each_scenario!(scenarios);

#[test]
fn probe_lists_registered_scenarios_for_python() {
    let output = process::checked_output(
        std::process::Command::new(env!("CARGO_BIN_EXE_client-probe")).arg("--list-scenarios"),
        "list-scenarios",
        5,
    );
    let names: Vec<_> = output.lines().collect();
    assert_eq!(names, egui_states_test_support::scenarios::SCENARIOS);
    assert!(!names.is_empty());
    assert_eq!(
        names.iter().collect::<std::collections::HashSet<_>>().len(),
        names.len(),
        "scenario names must be unique"
    );
}
