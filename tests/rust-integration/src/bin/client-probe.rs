fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 2 && args[1] == "--list-scenarios" {
        for scenario in egui_states_test_support::scenarios::SCENARIOS {
            println!("{scenario}");
        }
        return;
    }
    assert_eq!(
        args.len(),
        3,
        "Usage: client-probe PORT SCENARIO | --list-scenarios"
    );
    // Also bounds the executable when invoked directly by Cargo integration tests.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(25));
        eprintln!("client-probe exceeded its hard timeout");
        std::process::exit(124);
    });
    egui_states_test_support::scenarios::run(args[1].parse().unwrap(), &args[2]);
    println!("scenario {} passed", args[2]);
}
