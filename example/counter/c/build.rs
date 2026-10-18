// Compile the GUI's state source into the build script.
#[path = "../gui/src/lib.rs"]
#[allow(dead_code)]
mod state;

fn main() {
    println!("cargo:rerun-if-changed=../gui/src/lib.rs");
    println!("cargo:rerun-if-changed=bindings");
    egui_states::build_scripts::generate_c::<state::CounterState>("bindings").unwrap();
    // The smoke test asks `cc` for the compiler of this target.
    println!(
        "cargo:rustc-env=COUNTER_C_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
