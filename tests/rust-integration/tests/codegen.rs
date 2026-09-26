//! Rust consumer compilation and macro diagnostics belong to Cargo, not pytest.
use egui_states_test_process as process;

use std::{fs, path::PathBuf, process::Command, sync::Mutex};

// Serialize nested Cargo invocations even with Cargo's default parallel harness.
static CARGO: Mutex<()> = Mutex::new(());

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn cargo() -> Command {
    let mut command = Command::new(env!("CARGO"));
    command.current_dir(root());
    command
}

fn fixture_command(operation: &str) -> Command {
    // Place child builds beside the parent's profile artifacts, including when
    // Cargo uses a custom target directory or target triple. A separate directory
    // prevents recursive Cargo from waiting on the parent build lock.
    let profile = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let mut command = cargo();
    command
        .arg(operation)
        .arg("--manifest-path")
        .arg(root().join("tests/generated-code-consumer/Cargo.toml"))
        .arg("--target-dir")
        .arg(profile.join("test-codegen"));
    command
}

fn directory(name: &str) -> PathBuf {
    let path = root()
        .join("tests/build-artifacts")
        .join(format!("cargo-codegen-{}-{name}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn generated_rust_and_custom_derive_are_usable() {
    let _guard = CARGO.lock().unwrap_or_else(|e| e.into_inner());
    process::checked_output(
        fixture_command("test").args(["--bin", "codegen-probe"]),
        "rust-consumer",
        600,
    );
}

fn generate(path: &std::path::Path, reverse: bool) {
    let mut command = fixture_command("run");
    command.args(["--bin", "codegen-probe", "--"]).arg(path);
    if reverse {
        command.arg("reverse");
    }
    process::checked_output(&mut command, "generate-bindings", 600);
}

const FILES: [&str; 6] = [
    "rust/mod.rs",
    "rust/structs.rs",
    "rust/enums.rs",
    "python/__init__.py",
    "python/structs.py",
    "python/enums.py",
];

fn snapshot(path: &std::path::Path) -> Vec<Vec<u8>> {
    FILES
        .iter()
        .map(|file| fs::read(path.join(file)).unwrap())
        .collect()
}

#[test]
fn equivalent_inputs_preserve_all_six_files_and_unchanged_timestamps() {
    let _guard = CARGO.lock().unwrap_or_else(|e| e.into_inner());
    let first = directory("first");
    let second = directory("second");
    generate(&first, false);
    generate(&second, true);
    let before = snapshot(&first);
    assert_eq!(
        before,
        snapshot(&second),
        "equivalent input constructions changed output"
    );
    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(946684800);
    for name in FILES {
        fs::File::options()
            .write(true)
            .open(first.join(name))
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(old))
            .unwrap();
    }
    let timestamps: Vec<_> = FILES
        .iter()
        .map(|name| fs::metadata(first.join(name)).unwrap().modified().unwrap())
        .collect();
    generate(&first, true);
    assert_eq!(snapshot(&first), before);
    for (name, timestamp) in FILES.iter().zip(timestamps) {
        assert_eq!(
            fs::metadata(first.join(name)).unwrap().modified().unwrap(),
            timestamp,
            "rewrote unchanged {name}"
        );
    }
    fs::remove_dir_all(first).unwrap();
    fs::remove_dir_all(second).unwrap();
}

fn rejected(name: &str, diagnostic: &str) {
    let _guard = CARGO.lock().unwrap_or_else(|e| e.into_inner());
    let (status, output) = process::output(
        fixture_command("check").args(["--features", "invalid", "--bin", name]),
        name,
        600,
    );
    assert!(
        !status.success(),
        "unsupported input {name} compiled successfully; expected {diagnostic}"
    );
    assert!(
        output.contains(diagnostic),
        "missing diagnostic {diagnostic:?} for {name}:\n{output}"
    );
}

macro_rules! compile_fail {
    ($($test:ident: $name:literal => $diagnostic:literal),* $(,)?) => {$(
        #[test]
        fn $test() { rejected($name, $diagnostic); }
    )*};
}

compile_fail! {
    typed_generic: "typed-generic" => "Structs with generics are not supported",
    typed_tuple: "typed-tuple" => "Struct fields must be named",
    typed_payload_enum: "typed-payload-enum" => "Enum variants must be unit variants",
    typed_union: "typed-union" => "Unions are not supported",
    typed_option: "typed-option" => "unknown `typed` option",
    typed_duplicate_option: "typed-duplicate-option" => "duplicate `rust_derive(...)` option",
    state_generic: "state-generic" => "State derive does not support generics",
    state_empty: "state-empty" => "State derive requires at least one named field",
    atomic_payload: "atomic-payload" => "Enum variants must be unit variants",
    initial_tuple: "initial-tuple" => "Struct fields must be named",
    atomic_static_payload: "atomic-static-payload" => "Enum variants must be unit variants",
}

#[test]
fn conflicting_definitions_fail_the_consumer_build() {
    let _guard = CARGO.lock().unwrap_or_else(|e| e.into_inner());
    let (status, output) = process::output(
        fixture_command("check").args(["--features", "conflicting", "--bin", "codegen-probe"]),
        "conflicting-definitions",
        600,
    );
    assert!(
        !status.success(),
        "conflicting generated definitions compiled successfully"
    );
    assert!(
        output.contains("State Leaf defined multiple times with different fields"),
        "{output}"
    );
}
