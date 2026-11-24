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
    typed_empty_braces: "typed-empty-braces" => "has empty braces; write `Home` without them",
    typed_empty_parentheses: "typed-empty-parentheses" => "has empty parentheses; write `Reset` without them",
    typed_data_discriminant: "typed-data-discriminant" => "cannot have explicit discriminants",
    typed_recursive_enum: "typed-recursive-enum" => "Enum Invalid cannot contain itself",
    typed_recursive_struct: "typed-recursive-struct" => "Struct Invalid cannot contain itself",
    typed_empty_enum: "typed-empty-enum" => "Enum Invalid has no variants",
    atomic_empty_enum: "atomic-empty-enum" => "Enum Invalid has no variants",
    initial_empty_enum: "initial-empty-enum" => "Enum Invalid has no variants",
    typed_serde_variant_field: "typed-serde-variant-field" => "skip",
    typed_union: "typed-union" => "Unions are not supported",
    typed_option: "typed-option" => "unknown `typed` option",
    typed_duplicate_option: "typed-duplicate-option" => "duplicate `rust_derive(...)` option",
    state_generic: "state-generic" => "State derive does not support generics",
    state_empty: "state-empty" => "State derive requires at least one named field",
    state_low_priority_value: "state-low-priority-value" => "`low_priority` applies to Image, ImageMulti",
    state_option: "state-option" => "unknown `state` option",
    state_queue_image: "state-queue-image" => "`queue` applies to Value, ValueAtomic, Signal",
    state_queue_zero: "state-queue-zero" => "`queue = N` takes a capacity of at least 1",
    state_queue_capacity_value: "state-queue-capacity-value" => "`queue = N` applies to ValueTake, whose queue is on the client; `Value` takes `queue`",
    state_duplicate_queue: "state-duplicate-queue" => "duplicate `state` option",
    state_queue_suffix: "state-queue-suffix" => "`queue = N` takes a `usize`; write the capacity without the `u8` suffix",
    state_init_wrong_kind: "state-init-wrong-kind" => "`init` applies to Value, ValueAtomic, Static, StaticAtomic; not to `Signal`",
    state_duplicate_init: "state-duplicate-init" => "duplicate `state` option",
    state_empty_attribute: "state-empty-attribute" => "empty `state` attribute",
    state_struct_attribute: "state-struct-attribute" => "`state` attributes belong on fields, not on the struct",
    atomic_payload: "atomic-payload" => "#[derive(Atomic)] supports only fieldless enums",
    initial_tuple: "initial-tuple" => "Struct fields must be named",
    typed_discriminant_range: "typed-discriminant-range" => "Enum discriminants must fit in i32",
    typed_discriminant_expression: "typed-discriminant-expression" => "Enum discriminants must be integer literals",
    typed_discriminant_overflow: "typed-discriminant-overflow" => "Enum discriminants must fit in i32",
    // Serde attributes that change the encoding make `Typed` describe the wire
    // incorrectly; any rejection message naming the attribute satisfies these.
    typed_serde_skip: "typed-serde-skip" => "skip",
    typed_serde_flatten: "typed-serde-flatten" => "flatten",
    typed_serde_untagged: "typed-serde-untagged" => "untagged",
    // Behind `cfg_attr`, any Serde attribute is rejected: the type description
    // does not follow the condition.
    typed_serde_cfg_attr: "typed-serde-cfg-attr" => "a `serde` attribute inside `cfg_attr` is not supported by `#[typed]`",
    typed_serde_cfg_attr_nested: "typed-serde-cfg-attr-nested" => "a `serde` attribute inside `cfg_attr` is not supported by `#[typed]`",
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
