//! Builds the C counter server against the C ABI library and runs it with the
//! native client probe of the example smoke tests.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use egui_states_test_process as process;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn free_port() -> u16 {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Builds with Cargo and returns the JSON artifact line of target `name`.
fn build(arguments: &[&str], name: &str) -> String {
    let output = Command::new(env!("CARGO"))
        .arg("build")
        .args(arguments)
        .arg("--message-format=json-render-diagnostics")
        .current_dir(repository())
        .stderr(Stdio::inherit())
        .output()
        .unwrap();
    assert!(output.status.success(), "building {name} failed");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .rev()
        .find(|line| {
            line.contains("\"reason\":\"compiler-artifact\"")
                && line.contains(&format!("\"name\":\"{name}\""))
        })
        .unwrap_or_else(|| panic!("no {name} artifact"))
        .to_owned()
}

fn json_path(text: &str) -> PathBuf {
    PathBuf::from(text.trim_matches('"').replace("\\\\", "\\"))
}

/// Builds the client probe of the example smoke tests.
fn build_probe() -> PathBuf {
    let artifact = build(
        &["-p", "example_smoke_tests", "--bin", "example-probe"],
        "example-probe",
    );
    let start = artifact.find("\"executable\":").unwrap() + "\"executable\":".len();
    let end = start + artifact[start..].find([',', '}']).unwrap();
    json_path(&artifact[start..end])
}

/// Builds `egui_states_c` and returns the shared library and, on Windows,
/// its import library.
fn build_library() -> (PathBuf, Option<PathBuf>) {
    let artifact = build(&["-p", "egui_states_c"], "egui_states_c");
    let start = artifact.find("\"filenames\":[").unwrap() + "\"filenames\":[".len();
    let end = start + artifact[start..].find(']').unwrap();
    let files: Vec<PathBuf> = artifact[start..end].split(',').map(json_path).collect();
    let find = |suffix: &str| {
        files
            .iter()
            .find(|file| file.to_string_lossy().ends_with(suffix))
            .cloned()
    };
    let shared = [".so", ".dylib", ".dll"]
        .into_iter()
        .find_map(find)
        .unwrap_or_else(|| panic!("no shared library in {files:?}"));
    (shared, find(".dll.lib"))
}

fn compiler(cpp: bool) -> cc::Tool {
    let target = env!("COUNTER_C_TARGET");
    cc::Build::new()
        .cargo_metadata(false)
        .cpp(cpp)
        .opt_level(0)
        .target(target)
        .host(target)
        .try_get_compiler()
        .unwrap_or_else(|error| {
            panic!(
                "no C compiler found ({error}); install one or set \
                 EGUI_STATES_SKIP_C_TESTS=1 to skip this test"
            )
        })
}

fn add_includes(command: &mut Command, msvc: bool, includes: &[PathBuf]) {
    for include in includes {
        if msvc {
            command.arg(format!("/I{}", include.display()));
        } else {
            command.arg("-I").arg(include);
        }
    }
}

#[test]
fn c_counter_serves_the_counter_gui() {
    if std::env::var_os("EGUI_STATES_SKIP_C_TESTS").is_some() {
        eprintln!("skipped: EGUI_STATES_SKIP_C_TESTS is set");
        return;
    }
    let root = repository();
    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let (library, import_library) = build_library();
    let library_dir = library.parent().unwrap().to_path_buf();
    let probe = build_probe();
    let out_dir =
        std::env::temp_dir().join(format!("egui-states-c-counter-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let executable = out_dir.join(if cfg!(windows) {
        "counter-c.exe"
    } else {
        "counter-c"
    });
    let includes = [
        root.join("crates/egui-states-c/include"),
        example.join("bindings"),
    ];

    let c = compiler(false);
    let mut command = c.to_command();
    if c.is_like_msvc() {
        command.args(["/nologo", "/W4", "/WX"]);
        add_includes(&mut command, true, &includes);
        command
            .arg(example.join("main.c"))
            .arg(format!("/Fe{}", executable.display()))
            .arg(format!("/Fo{}\\", out_dir.display()))
            .arg(import_library.expect("import library of egui_states_c"));
    } else {
        command.args(["-std=c99", "-Wall", "-Wextra", "-Werror", "-pedantic"]);
        add_includes(&mut command, false, &includes);
        command
            .arg(example.join("main.c"))
            .arg(&library)
            .arg(format!("-Wl,-rpath,{}", library_dir.display()))
            .arg("-o")
            .arg(&executable);
    }
    process::checked_output(&mut command, "compile counter-c", 120);

    // Both headers must also be valid C++.
    let cpp = compiler(true);
    let mut command = cpp.to_command();
    if cpp.is_like_msvc() {
        command.args(["/nologo", "/W4", "/WX", "/Zs", "/TP"]);
        add_includes(&mut command, true, &includes);
    } else {
        command.args([
            "-std=c++11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-pedantic",
            "-fsyntax-only",
        ]);
        add_includes(&mut command, false, &includes);
    }
    command.arg(example.join("header_check.cpp"));
    process::checked_output(&mut command, "check headers as C++", 120);

    let port = free_port();
    let mut path = std::env::var_os("PATH").unwrap_or_default();
    if cfg!(windows) {
        let mut paths = vec![library_dir.clone()];
        paths.extend(std::env::split_paths(&path));
        path = std::env::join_paths(paths).unwrap();
    }
    let mut server = Command::new(&executable)
        .arg(port.to_string())
        .arg("--smoke-test")
        .env("PATH", path)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(server.stdout.take().unwrap()).lines();
    assert_eq!(
        lines.next().unwrap().unwrap(),
        format!("listening {port}"),
        "counter-c did not start"
    );

    process::checked_output(
        Command::new(&probe).arg(port.to_string()).arg("counter"),
        "counter-c probe",
        30,
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = server.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            server.kill().unwrap();
            panic!("counter-c did not exit after the client disconnected");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output: Vec<String> = lines.map(Result::unwrap).collect();
    assert!(
        status.success(),
        "counter-c exited with {status}: {output:?}"
    );
    assert_eq!(output, ["count: 17", "count: 42"]);
    let _ = std::fs::remove_dir_all(out_dir);
}
