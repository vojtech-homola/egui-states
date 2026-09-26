#[path = "schema.rs"]
mod schema;
fn main() {
    println!("cargo:rerun-if-changed=schema.rs");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    egui_states::build_scripts::generate_rust::<schema::Root<false>>(out.join("bindings")).unwrap();
    #[cfg(feature = "conflicting")]
    egui_states::build_scripts::generate_rust::<schema::conflicting::Conflict>(
        out.join("conflict"),
    )
    .unwrap();
    std::fs::write(
        out.join("module.rs"),
        format!("#[path = {:?}] mod bindings;", out.join("bindings/mod.rs")),
    )
    .unwrap();
}
