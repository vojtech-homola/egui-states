fn main() {
    // Lets plain `cargo build` link the extension on macOS, as maturin does.
    pyo3_build_config::add_extension_module_link_args();
}
