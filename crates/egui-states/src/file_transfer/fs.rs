//! Native file sources and sinks.
//!
//! Disk calls block, so they run on Tokio's blocking threads and in blocks
//! larger than a chunk: the credit window still decides what is sent.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use crate::file_transfer::TransferError;
use crate::protocol::file_transfer::FileEntry;

/// Bytes read from or written to disk at once.
const BLOCK: usize = 1024 * 1024;

/// The files of an outgoing transfer, read one after another.
pub(crate) struct FilesSource {
    files: Vec<(PathBuf, u64)>,
    next: usize,
    open: Option<(File, u64)>,
    buffer: Vec<u8>,
    position: usize,
}

impl FilesSource {
    /// Reads `files`, each expected to have its size.
    pub(crate) fn new(files: Vec<(PathBuf, u64)>) -> Self {
        Self {
            files,
            next: 0,
            open: None,
            buffer: Vec::new(),
            position: 0,
        }
    }

    pub(crate) async fn read(&mut self, len: usize) -> Result<Vec<u8>, String> {
        if self.buffer.len() - self.position < len {
            let mut state = std::mem::replace(self, Self::new(Vec::new()));
            let (state_back, result) = tokio::task::spawn_blocking(move || {
                let result = state.fill(len.max(BLOCK));
                (state, result)
            })
            .await
            .map_err(|error| format!("reading failed: {error}"))?;
            *self = state_back;
            result?;
        }
        let end = self.position + len;
        if end > self.buffer.len() {
            return Err("the files are shorter than announced".to_string());
        }
        let chunk = self.buffer[self.position..end].to_vec();
        self.position = end;
        Ok(chunk)
    }

    /// Reads ahead until `want` bytes are buffered or the files end.
    fn fill(&mut self, want: usize) -> Result<(), String> {
        self.buffer.drain(..self.position);
        self.position = 0;
        while self.buffer.len() < want {
            if self.open.is_none() {
                let Some((path, size)) = self.files.get(self.next) else {
                    return Ok(());
                };
                self.next += 1;
                let file = File::open(path)
                    .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
                let actual = file
                    .metadata()
                    .map_err(|error| format!("cannot read {}: {error}", path.display()))?
                    .len();
                if actual != *size {
                    return Err(format!(
                        "{} changed size after the offer: {size} bytes announced, {actual} now",
                        path.display()
                    ));
                }
                self.open = Some((file, *size));
            }
            let (file, left) = self.open.as_mut().expect("opened above");
            let take = (*left).min((want - self.buffer.len()) as u64) as usize;
            let start = self.buffer.len();
            self.buffer.resize(start + take, 0);
            file.read_exact(&mut self.buffer[start..])
                .map_err(|error| format!("reading failed: {error}"))?;
            *left -= take as u64;
            if *left == 0 {
                self.open = None;
            }
        }
        Ok(())
    }
}

/// Where a received transfer is saved.
pub(crate) enum Target {
    /// The single file of the offer, under this path.
    File(PathBuf),
    /// Every file of the offer, under its own name in this folder.
    Dir(PathBuf),
}

/// Writes the files of an incoming transfer.
///
/// Each file goes to a temporary file next to its target and is renamed over
/// the target once complete, so a failed transfer never leaves half a file.
/// Files finished before a failure stay. Dropping the sink before it finished
/// removes the temporary file.
pub(crate) struct FileSink {
    targets: Vec<(PathBuf, u64, Option<SystemTime>)>,
    /// For a folder target, the folder the receiver chose: nothing below it
    /// may be a symbolic link, or a file could land outside of it.
    root: Option<PathBuf>,
    next: usize,
    open: Option<OpenFile>,
}

struct OpenFile {
    file: File,
    temp: PathBuf,
    target: PathBuf,
    left: u64,
    modified: Option<SystemTime>,
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl FileSink {
    /// No disk access happens here, so it may run on the UI thread.
    pub(crate) fn new(target: Target, files: &[FileEntry]) -> Self {
        let targets = files
            .iter()
            .map(|file| {
                let path = match &target {
                    Target::File(path) => path.clone(),
                    Target::Dir(dir) => file
                        .name
                        .split('/')
                        .fold(dir.clone(), |path, part| path.join(part)),
                };
                let modified = file.modified.and_then(time_from_millis);
                (path, file.size, modified)
            })
            .collect();
        let root = match target {
            Target::File(_) => None,
            Target::Dir(dir) => Some(dir),
        };
        Self {
            targets,
            root,
            next: 0,
            open: None,
        }
    }

    /// Creates the folders between the chosen folder and `target`, refusing
    /// to follow a symbolic link or junction on the way.
    fn create_parents(&self, target: &Path) -> io::Result<()> {
        let Some(root) = &self.root else {
            let parent = target
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            return std::fs::create_dir_all(parent);
        };
        std::fs::create_dir_all(root)?;
        let relative = target.strip_prefix(root).map_err(io::Error::other)?;
        let mut path = root.clone();
        let components: Vec<_> = relative.components().collect();
        for component in &components[..components.len().saturating_sub(1)] {
            path.push(component);
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(io::Error::other(format!(
                        "{} is a symbolic link; files are not saved through it",
                        path.display()
                    )));
                }
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => {
                    return Err(io::Error::other(format!(
                        "{} is not a folder",
                        path.display()
                    )));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    std::fs::create_dir(&path)?;
                }
                Err(error) => return Err(error),
            }
        }
        if std::fs::symlink_metadata(target).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(io::Error::other(format!(
                "{} is a symbolic link; it is not replaced",
                target.display()
            )));
        }
        Ok(())
    }

    /// The folder or file the transfer is saved to.
    #[cfg_attr(not(any(feature = "client", feature = "server")), allow(dead_code))]
    pub(crate) fn display_path(target: &Target) -> PathBuf {
        match target {
            Target::File(path) | Target::Dir(path) => path.clone(),
        }
    }

    /// Writes the next bytes of the concatenated files.
    pub(crate) fn write(&mut self, mut data: &[u8]) -> io::Result<()> {
        loop {
            // Empty files need no data; create them as the stream passes them.
            self.open_next_if_needed()?;
            let Some(open) = self.open.as_mut() else {
                return if data.is_empty() {
                    Ok(())
                } else {
                    Err(io::Error::other("more data than the files hold"))
                };
            };
            if data.is_empty() && open.left > 0 {
                return Ok(());
            }
            let take = (open.left as usize).min(data.len());
            open.file.write_all(&data[..take])?;
            open.left -= take as u64;
            data = &data[take..];
            if open.left == 0 {
                self.complete()?;
            }
        }
    }

    /// Completes the files that need no more data; call after the last byte.
    pub(crate) fn finish(&mut self) -> io::Result<()> {
        self.write(&[])?;
        if self.open.is_some() || self.next < self.targets.len() {
            return Err(io::Error::other("the data ended before the files"));
        }
        Ok(())
    }

    fn open_next_if_needed(&mut self) -> io::Result<()> {
        while self.open.is_none() && self.next < self.targets.len() {
            let (target, size, modified) = self.targets[self.next].clone();
            self.next += 1;
            self.create_parents(&target)?;
            let parent = target
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let name = target
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let temp = parent.join(format!(
                ".{name}.{}-{}.part",
                std::process::id(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            self.open = Some(OpenFile {
                file,
                temp,
                target,
                left: size,
                modified,
            });
            if size == 0 {
                self.complete()?;
            }
        }
        Ok(())
    }

    fn complete(&mut self) -> io::Result<()> {
        let open = self.open.take().expect("a file is open");
        let result = (|| {
            open.file.sync_data()?;
            if let Some(modified) = open.modified {
                open.file.set_modified(modified)?;
            }
            drop(open.file);
            std::fs::rename(&open.temp, &open.target)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&open.temp);
        }
        result
    }
}

impl Drop for FileSink {
    fn drop(&mut self) {
        if let Some(open) = self.open.take() {
            drop(open.file);
            let _ = std::fs::remove_file(&open.temp);
        }
    }
}

/// The size and offer entry of the file at `path`.
pub(crate) fn stat(path: &Path, name: String) -> Result<(FileEntry, u64), TransferError> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| TransferError::Io(format!("{}: {error}", path.display())))?;
    if !metadata.is_file() {
        return Err(TransferError::Io(format!(
            "{} is not a file",
            path.display()
        )));
    }
    let size = metadata.len();
    let entry = FileEntry {
        name,
        size,
        modified: modified_millis(&metadata),
    };
    Ok((entry, size))
}

pub(crate) fn file_name(path: &Path) -> Result<String, TransferError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| {
            TransferError::InvalidInput(format!("{} has no UTF-8 file name", path.display()))
        })
}

/// The modification time of `path` in milliseconds since the Unix epoch.
pub(crate) fn modified_millis(metadata: &std::fs::Metadata) -> Option<i64> {
    let modified = metadata.modified().ok()?;
    match modified.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).ok(),
        Err(before) => i64::try_from(before.duration().as_millis())
            .ok()
            .map(|ms| -ms),
    }
}

pub(crate) fn time_from_millis(millis: i64) -> Option<SystemTime> {
    let offset = Duration::from_millis(millis.unsigned_abs());
    if millis >= 0 {
        SystemTime::UNIX_EPOCH.checked_add(offset)
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "egui_states_fs_{name}_{}_{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn entry(name: &str, size: u64, modified: Option<i64>) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            size,
            modified,
        }
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names = Vec::new();
        for entry in walk(dir) {
            names.push(
                entry
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
        names.sort();
        names
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn a_sink_writes_files_atomically_with_their_times() {
        let dir = temp_dir("sink");
        let files = [
            entry("a.bin", 3, Some(1_000_000)),
            entry("sub/empty", 0, None),
            entry("sub/deeper/b.bin", 4, Some(-5_000)),
        ];
        let mut sink = FileSink::new(Target::Dir(dir.clone()), &files);
        sink.write(b"ab").unwrap();
        assert_eq!(entries(&dir).len(), 1, "only the temporary file");
        sink.write(b"cde").unwrap();
        sink.write(b"fg").unwrap();
        sink.finish().unwrap();
        assert_eq!(entries(&dir), ["a.bin", "sub/deeper/b.bin", "sub/empty"]);
        assert_eq!(std::fs::read(dir.join("a.bin")).unwrap(), b"abc");
        assert_eq!(
            std::fs::read(dir.join("sub/deeper/b.bin")).unwrap(),
            b"defg"
        );
        let modified = std::fs::metadata(dir.join("a.bin"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(modified, time_from_millis(1_000_000).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_dropped_sink_keeps_finished_files_and_removes_the_partial_one() {
        let dir = temp_dir("partial");
        let files = [entry("a", 2, None), entry("b", 10, None)];
        let mut sink = FileSink::new(Target::Dir(dir.clone()), &files);
        sink.write(b"xyz").unwrap();
        drop(sink);
        assert_eq!(entries(&dir), ["a"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_target_replaces_an_existing_file() {
        let dir = temp_dir("replace");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("target.txt");
        std::fs::write(&path, b"old contents").unwrap();
        let mut sink = FileSink::new(Target::File(path.clone()), &[entry("x", 3, None)]);
        sink.write(b"new").unwrap();
        sink.finish().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(entries(&dir), ["target.txt"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_target_does_not_follow_symbolic_links() {
        let dir = temp_dir("links");
        let outside = temp_dir("outside");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("link")).unwrap();
        std::os::unix::fs::symlink(outside.join("target"), dir.join("direct")).unwrap();

        let mut sink = FileSink::new(Target::Dir(dir.clone()), &[entry("link/file", 3, None)]);
        let error = sink.write(b"abc").unwrap_err();
        assert!(error.to_string().contains("symbolic link"), "{error}");
        let mut sink = FileSink::new(Target::Dir(dir.clone()), &[entry("direct", 3, None)]);
        assert!(sink.write(b"abc").is_err());
        drop(sink);
        assert!(
            std::fs::read_dir(&outside).unwrap().next().is_none(),
            "nothing outside"
        );

        // The chosen folder itself may be a link: the user picked it.
        let chosen = temp_dir("chosen");
        std::os::unix::fs::symlink(&outside, &chosen).unwrap();
        let mut sink = FileSink::new(Target::Dir(chosen.clone()), &[entry("sub/a", 1, None)]);
        sink.write(b"x").unwrap();
        sink.finish().unwrap();
        assert_eq!(std::fs::read(outside.join("sub/a")).unwrap(), b"x");
        let _ = std::fs::remove_file(&chosen);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn too_much_or_too_little_data_is_an_error() {
        let dir = temp_dir("sizes");
        let mut sink = FileSink::new(Target::Dir(dir.clone()), &[entry("a", 2, None)]);
        assert!(sink.write(b"abc").is_err());
        let mut sink = FileSink::new(Target::Dir(dir.clone()), &[entry("b", 2, None)]);
        sink.write(b"a").unwrap();
        assert!(sink.finish().is_err());
        drop(sink);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_source_reads_files_in_order_and_notices_a_changed_size() {
        let dir = temp_dir("source");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a"), b"hello").unwrap();
        std::fs::write(dir.join("b"), b"").unwrap();
        std::fs::write(dir.join("c"), b" world").unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut source = FilesSource::new(vec![
                (dir.join("a"), 5),
                (dir.join("b"), 0),
                (dir.join("c"), 6),
            ]);
            assert_eq!(source.read(3).await.unwrap(), b"hel");
            assert_eq!(source.read(8).await.unwrap(), b"lo world");

            let mut changed = FilesSource::new(vec![(dir.join("a"), 4)]);
            assert!(changed.read(4).await.unwrap_err().contains("changed size"));
            let mut missing = FilesSource::new(vec![(dir.join("nope"), 1)]);
            assert!(missing.read(1).await.unwrap_err().contains("cannot open"));
        });
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
