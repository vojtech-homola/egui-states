//! Failure containment for tests of potentially blocking native operations.

pub(crate) fn isolated() -> bool {
    let name = std::thread::current().name().unwrap().to_owned();
    if std::env::var("EGUI_TEST_CHILD").as_deref() == Ok(&name) {
        return false;
    }
    let path = std::env::temp_dir().join(format!(
        "egui-test-{}-{}.log",
        std::process::id(),
        name.replace(':', "_")
    ));
    let log = std::fs::File::create(&path).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture"])
        .env("EGUI_TEST_CHILD", &name)
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let output = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(path);
    assert!(
        status.is_some_and(|s| s.success()),
        "{name}: child failed or exceeded 25s\n{output}"
    );
    true
}

#[cfg(feature = "client")]
pub(crate) fn ack(
    receiver: &mut tokio::sync::mpsc::UnboundedReceiver<
        Option<crate::client::messages::ChannelMessage>,
    >,
    id: u64,
) {
    assert!(
        matches!(receiver.try_recv(), Ok(Some(crate::client::messages::ChannelMessage::Ack(actual))) if actual == id),
        "expected ACK for {id}"
    );
    assert!(receiver.try_recv().is_err(), "unexpected extra message");
}

#[cfg(feature = "client")]
pub(crate) fn texture_updates(
    context: &egui::Context,
) -> std::collections::HashMap<egui::TextureId, Vec<egui::epaint::ImageDelta>> {
    let mut delta = context.tex_manager().write().take_delta();
    let updates = delta
        .set
        .drain()
        .map(|(id, changes)| (id, changes.into_vec()))
        .collect();
    // Headless tests consume the deltas themselves instead of forwarding them to a painter.
    delta.clear();
    updates
}
