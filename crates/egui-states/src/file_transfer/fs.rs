//! Native file sources and sinks.
//!
//! Disk calls block, so they run on Tokio's blocking threads and in blocks
//! larger than a chunk: the credit window still decides what is sent.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ffi::{OsStr, OsString};
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
/// The rename replaces whatever has the name, a symbolic link included, and
/// never writes through it. Files finished before a failure stay. Dropping
/// the sink before it finished removes the temporary file.
pub(crate) struct FileSink {
    targets: Vec<(PathBuf, u64, Option<SystemTime>)>,
    /// For a folder target, the folder the receiver chose: no folder below it
    /// may be a symbolic link, or a file could land outside of it.
    root: Option<PathBuf>,
    /// What is known of the folders below `root`; each is looked at once.
    folders: Folders,
    next: usize,
    open: Option<OpenFile>,
}

/// The folders below the chosen one that were looked at.
///
/// A folder is kept as its name and the number of its parent, not as its
/// path, so what is remembered grows with the offer and not with the depth of
/// its folders.
#[derive(Default)]
struct Folders {
    /// The number of the folder with a name in a parent; the chosen folder
    /// is the parent `None`.
    numbers: HashMap<(Option<usize>, OsString), usize>,
    /// By number, whether the folder is on disk as a real folder; otherwise
    /// nothing was there.
    exists: Vec<bool>,
}

impl Folders {
    /// The number of the folder `name` in `parent`. A folder seen for the
    /// first time exists if `look` says so.
    fn number(
        &mut self,
        parent: Option<usize>,
        name: &OsStr,
        look: impl FnOnce() -> io::Result<bool>,
    ) -> io::Result<usize> {
        match self.numbers.entry((parent, name.to_os_string())) {
            Entry::Occupied(known) => Ok(*known.get()),
            Entry::Vacant(new) => {
                self.exists.push(look()?);
                Ok(*new.insert(self.exists.len() - 1))
            }
        }
    }
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
                    Target::Dir(dir) => {
                        let mut path = dir.clone();
                        path.extend(file.name.split('/'));
                        path
                    }
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
            folders: Folders::default(),
            next: 0,
            open: None,
        }
    }

    /// Looks at the folders that already exist below the chosen one, for
    /// every file, before the first is written: a link in the way of a later
    /// file fails the transfer at its start, not halfway.
    fn check_existing(&mut self) -> io::Result<()> {
        let Some(root) = &self.root else {
            return Ok(());
        };
        std::fs::create_dir_all(root)?;
        for (target, _, _) in &self.targets {
            let mut path = root.clone();
            let mut parent = None;
            for name in folder_names(root, target)? {
                path.push(name);
                let folder = self.folders.number(parent, name, || is_folder(&path))?;
                // Nothing exists below a folder that is missing.
                if !self.folders.exists[folder] {
                    break;
                }
                parent = Some(folder);
            }
        }
        Ok(())
    }

    /// Creates the folders between the chosen folder and `target`, refusing
    /// to follow a symbolic link or junction on the way.
    fn create_parents(&mut self, target: &Path) -> io::Result<()> {
        let Some(root) = &self.root else {
            return std::fs::create_dir_all(parent_of(target));
        };
        let mut path = root.clone();
        let mut parent = None;
        for name in folder_names(root, target)? {
            path.push(name);
            let folder = self.folders.number(parent, name, || Ok(false))?;
            if !self.folders.exists[folder] {
                ensure_folder(&path)?;
                self.folders.exists[folder] = true;
            }
            parent = Some(folder);
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
            if self.next == 0 {
                self.check_existing()?;
            }
            let (target, size, modified) = self.targets[self.next].clone();
            self.next += 1;
            self.create_parents(&target)?;
            let temp = parent_of(&target).join(temp_name(&target));
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

/// How much of a target's name its temporary file keeps.
const TEMP_NAME_KEPT: usize = 96;

/// The name of the temporary file of `target`: hidden, unique in this
/// process, and recognisable by the start of the target's name. Only the
/// start, so a name near the file system's limit still has a temporary file.
fn temp_name(target: &Path) -> String {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let mut kept = name.len().min(TEMP_NAME_KEPT);
    while !name.is_char_boundary(kept) {
        kept -= 1;
    }
    format!(
        ".{}.{}-{}.part",
        &name[..kept],
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// The folder `target` is in.
fn parent_of(target: &Path) -> &Path {
    target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// The names of the folders between `root` and `target`, outermost first.
fn folder_names<'a>(
    root: &Path,
    target: &'a Path,
) -> io::Result<impl Iterator<Item = &'a OsStr> + use<'a>> {
    let relative = target.strip_prefix(root).map_err(io::Error::other)?;
    let folders = relative.parent().into_iter().flat_map(Path::components);
    Ok(folders.map(|component| component.as_os_str()))
}

/// Whether `path` is a folder, `false` if nothing is there. A symbolic link
/// or junction, even one to a folder, is an error, and so is a file.
fn is_folder(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::other(format!(
            "{} is a symbolic link; files are not saved through it",
            path.display()
        ))),
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(io::Error::other(format!(
            "{} is not a folder",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Makes `path` a folder unless it is one. A folder that appears between the
/// look and the creation, as when two transfers save into the same place, is
/// looked at like one that was there.
fn ensure_folder(path: &Path) -> io::Result<()> {
    if is_folder(path)? {
        return Ok(());
    }
    match std::fs::create_dir(path) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => match is_folder(path)? {
            true => Ok(()),
            false => Err(error),
        },
        result => result,
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

        // The link is in the way of the second file only, and still nothing
        // is written.
        let files = [entry("first", 1, None), entry("link/file", 3, None)];
        let mut sink = FileSink::new(Target::Dir(dir.clone()), &files);
        let error = sink.write(b"a").unwrap_err();
        assert!(error.to_string().contains("symbolic link"), "{error}");
        drop(sink);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "only the link");
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

    /// A link where a file goes is replaced like a file, by either target:
    /// the rename puts the new file in its place and never writes through it.
    #[cfg(unix)]
    #[test]
    fn a_link_in_the_place_of_a_file_is_replaced() {
        let dir = temp_dir("replaced_link");
        let outside = temp_dir("replaced_outside");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("kept"), b"old").unwrap();
        for name in ["in_folder", "as_file"] {
            std::os::unix::fs::symlink(outside.join("kept"), dir.join(name)).unwrap();
        }

        let mut sink = FileSink::new(Target::Dir(dir.clone()), &[entry("in_folder", 3, None)]);
        sink.write(b"new").unwrap();
        sink.finish().unwrap();
        let target = Target::File(dir.join("as_file"));
        let mut sink = FileSink::new(target, &[entry("x", 3, None)]);
        sink.write(b"new").unwrap();
        sink.finish().unwrap();

        for name in ["in_folder", "as_file"] {
            let metadata = std::fs::symlink_metadata(dir.join(name)).unwrap();
            assert!(metadata.is_file(), "{name} is a file now");
            assert_eq!(std::fs::read(dir.join(name)).unwrap(), b"new");
        }
        assert_eq!(std::fs::read(outside.join("kept")).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// Two transfers into the same place create the same folders; the one
    /// that comes second takes the folder the first made.
    #[test]
    fn a_folder_another_transfer_created_is_used() {
        let dir = temp_dir("shared");
        let files = |name: &str| [entry(name, 1, None), entry(&format!("run/{name}"), 1, None)];
        let mut first = FileSink::new(Target::Dir(dir.clone()), &files("a"));
        let mut second = FileSink::new(Target::Dir(dir.clone()), &files("b"));
        // Both look before `run` exists.
        first.write(b"1").unwrap();
        second.write(b"1").unwrap();
        first.write(b"2").unwrap();
        second.write(b"2").unwrap();
        first.finish().unwrap();
        second.finish().unwrap();
        assert_eq!(entries(&dir), ["a", "b", "run/a", "run/b"]);

        ensure_folder(&dir.join("run")).unwrap();
        ensure_folder(&dir.join("new")).unwrap();
        assert!(dir.join("new").is_dir());
        let error = ensure_folder(&dir.join("a")).unwrap_err();
        assert!(error.to_string().contains("not a folder"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn deep_folders_are_looked_at_once() {
        let dir = temp_dir("deep");
        std::fs::create_dir_all(dir.join("d/d")).unwrap();
        let deep = "d/".repeat(40);
        let files = [
            entry(&format!("{deep}a"), 1, None),
            entry("d/b", 1, None),
            entry(&format!("{deep}c"), 1, None),
        ];
        let mut sink = FileSink::new(Target::Dir(dir.clone()), &files);
        sink.write(b"123").unwrap();
        sink.finish().unwrap();
        // Each folder is remembered once, whether it was there or created.
        assert_eq!(sink.folders.exists, [true; 40]);
        assert_eq!(
            entries(&dir),
            ["d/b".to_string(), format!("{deep}a"), format!("{deep}c")]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_temporary_name_keeps_only_the_start_of_a_long_name() {
        let temp = temp_name(Path::new("dir/run.tiff"));
        assert!(temp.starts_with(".run.tiff."), "{temp}");
        assert!(temp.ends_with(".part"), "{temp}");
        assert_ne!(temp, temp_name(Path::new("dir/run.tiff")));

        // 255 bytes are the limit of the common file systems.
        for name in ["a".repeat(255), "\u{17e}".repeat(127)] {
            let temp = temp_name(Path::new(&name));
            assert!(temp.len() < 160, "{temp}");
            assert!(temp.starts_with(&format!(".{}", &name[..2])), "{temp}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_name_at_the_length_limit_is_saved() {
        let dir = temp_dir("long_name");
        let name = "a".repeat(255);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(&name), b"old").unwrap();
        let mut sink = FileSink::new(Target::Dir(dir.clone()), &[entry(&name, 3, None)]);
        sink.write(b"new").unwrap();
        sink.finish().unwrap();
        assert_eq!(std::fs::read(dir.join(&name)).unwrap(), b"new");
        assert_eq!(entries(&dir), [name]);
        std::fs::remove_dir_all(&dir).unwrap();
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
