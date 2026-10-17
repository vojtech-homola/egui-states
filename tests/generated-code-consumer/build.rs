#[cfg(feature = "awkward")]
#[path = "awkward.rs"]
mod awkward;
#[path = "schema.rs"]
mod schema;
fn main() {
    println!("cargo:rerun-if-changed=schema.rs");
    println!("cargo:rerun-if-changed=awkward.rs");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    egui_states::build_scripts::generate_rust::<schema::Root>(out.join("bindings")).unwrap();
    #[cfg(feature = "conflicting")]
    egui_states::build_scripts::generate_rust::<schema::conflicting::Conflict>(
        out.join("conflict"),
    )
    .unwrap();
    #[cfg(feature = "awkward")]
    egui_states::build_scripts::generate_rust::<awkward::Awkward>(out.join("awkward")).unwrap();
    let mut module = format!("#[path = {:?}] mod bindings;", out.join("bindings/mod.rs"));
    if cfg!(feature = "awkward") {
        module.push_str(&format!(
            "\n#[path = {:?}] mod awkward_bindings;",
            out.join("awkward/mod.rs")
        ));
    }
    std::fs::write(out.join("module.rs"), module).unwrap();
}
