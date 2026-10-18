//! Keeps the checked-in C header equal to the one cbindgen generates.

use std::path::Path;

#[test]
fn header_matches_the_exported_functions() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config = cbindgen::Config::from_file(crate_dir.join("cbindgen.toml")).unwrap();
    let bindings = cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen failed to generate the header");
    let mut generated = Vec::new();
    bindings.write(&mut generated);
    let generated = String::from_utf8(generated).unwrap().replace("\r\n", "\n");

    let path = crate_dir.join("include").join("egui_states.h");
    if std::env::var_os("EGUI_STATES_BLESS").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let current = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        current == generated,
        "include/egui_states.h is out of date; regenerate it with\n\
         EGUI_STATES_BLESS=1 cargo test -p egui_states_c --test header"
    );
}
