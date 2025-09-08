use std::fs;
use std::io::Write;

fn create_version_file() {
    let lines: Vec<String> = fs::read_to_string("../../Cargo.toml")
        .unwrap()
        .lines()
        .map(String::from)
        .collect();

    for line in lines {
        if line.contains("version") {
            let mut version_file = fs::File::create("../../egui_pysync/version.py").unwrap();
            let version = line.replace("version", "VERSION");
            version_file.write_all(version.as_bytes()).unwrap();
            version_file.write_all(b"\n").unwrap();
            version_file.write_all(b"__version__ = VERSION\n").unwrap();
            return;
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=../../Cargo.toml");
    println!("cargo:rerun-if-changed=../../egui_pysync/version.py");

    create_version_file();
}
