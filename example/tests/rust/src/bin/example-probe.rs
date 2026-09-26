fn main() {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(25));
        eprintln!("example-probe exceeded its hard timeout");
        std::process::exit(124);
    });
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3, "Usage: example-probe PORT showcase|counter");
    let port = args[1].parse().unwrap();
    match args[2].as_str() {
        "showcase" => example_smoke_tests::showcase(port),
        "counter" => example_smoke_tests::counter(port),
        _ => panic!("unknown example"),
    }
}
