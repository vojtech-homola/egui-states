//! Shared only by test packages, never linked into the library.
pub fn output(
    command: &mut std::process::Command,
    label: &str,
    seconds: u64,
) -> (std::process::ExitStatus, String) {
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };
    let dir = std::env::temp_dir().join(format!("egui-probe-{}-{}", std::process::id(), label));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("output.log");
    let log = fs::File::create(&path).unwrap();
    let mut child = command
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            break None;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let output = fs::read_to_string(&path).unwrap();
    assert!(
        status.is_some(),
        "{label}: exceeded {seconds}s; output saved at {}\n{output}",
        path.display()
    );
    if status.unwrap().success() {
        let _ = fs::remove_dir_all(dir);
    }
    (status.unwrap(), output)
}

pub fn checked_output(command: &mut std::process::Command, label: &str, seconds: u64) -> String {
    let (status, output) = output(command, label, seconds);
    assert!(status.success(), "{label}: exited with {status}\n{output}");
    output
}
