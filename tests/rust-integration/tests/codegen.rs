//! Rust consumer compilation and macro diagnostics belong to Cargo, not pytest.
use egui_states_test_process as process;

use std::{path::PathBuf, process::Command, sync::Mutex};

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

#[test]
fn generated_rust_and_custom_derive_are_usable() {
    let _guard = CARGO.lock().unwrap_or_else(|e| e.into_inner());
    process::checked_output(
        fixture_command("test").args(["--bin", "codegen-probe"]),
        "rust-consumer",
        600,
    );
}

/// Target-language keywords, escaped strings, non-finite floats, and a unit
/// struct must survive generation as compilable, value-preserving Rust.
#[test]
fn awkward_names_and_literals_compile_as_rust_bindings() {
    let _guard = CARGO.lock().unwrap_or_else(|e| e.into_inner());
    process::checked_output(
        fixture_command("test").args(["--features", "awkward", "--bin", "codegen-probe"]),
        "awkward-rust-consumer",
        600,
    );
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
    typed_discriminant_range: "typed-discriminant-range" => "Enum discriminants must fit in i32",
    typed_discriminant_expression: "typed-discriminant-expression" => "Enum discriminants must be integer literals",
    typed_discriminant_overflow: "typed-discriminant-overflow" => "Enum discriminants must fit in i32",
    // Serde attributes that change the encoding make `Typed` describe the wire
    // incorrectly; any rejection message naming the attribute satisfies these.
    typed_serde_skip: "typed-serde-skip" => "skip",
    typed_serde_flatten: "typed-serde-flatten" => "flatten",
    typed_serde_untagged: "typed-serde-untagged" => "untagged",
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
