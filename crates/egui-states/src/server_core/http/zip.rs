//! A zip written as a stream, for browser downloads of several files.
//!
//! Entries are stored, not compressed: measurement data rarely compresses
//! well, and the archive stays as fast to write as the files are to read.
//! Each entry's checksum is only known after its data, so it follows the data
//! in a data descriptor. Every record uses the zip64 form, so archives and
//! entries can exceed 4 GB, and the total size is known before the first
//! byte, which gives the response its `Content-Length`.

use crate::protocol::file_transfer::FileEntry;

const LOCAL_HEADER: u32 = 0x0403_4b50;
const DATA_DESCRIPTOR: u32 = 0x0807_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const ZIP64_END: u32 = 0x0606_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const END: u32 = 0x0605_4b50;

/// Version 4.5, the first with zip64.
const VERSION: u16 = 45;
/// Bit 3: sizes and checksum follow the data; bit 11: UTF-8 names.
const FLAGS: u16 = (1 << 3) | (1 << 11);

const LOCAL_FIXED: u64 = 30 + 20;
const DESCRIPTOR_LEN: u64 = 24;
const CENTRAL_FIXED: u64 = 46 + 28;
const TAIL_LEN: u64 = 56 + 20 + 22;

struct Entry {
    name: Vec<u8>,
    size: u64,
    time: u16,
    date: u16,
}

/// The layout of the zip of a set of files.
pub(crate) struct ZipLayout {
    entries: Vec<Entry>,
}

impl ZipLayout {
    pub(crate) fn new(files: &[FileEntry]) -> Self {
        let entries = files
            .iter()
            .map(|file| {
                let (date, time) = file.modified.map_or((DOS_EPOCH_DATE, 0), dos_date_time);
                Entry {
                    name: file.name.as_bytes().to_vec(),
                    size: file.size,
                    time,
                    date,
                }
            })
            .collect();
        Self { entries }
    }

    /// The size of the whole archive.
    pub(crate) fn total_size(&self) -> u64 {
        let entries: u64 = self
            .entries
            .iter()
            .map(|e| LOCAL_FIXED + e.name.len() as u64 + e.size + DESCRIPTOR_LEN)
            .sum();
        let central: u64 = self
            .entries
            .iter()
            .map(|e| CENTRAL_FIXED + e.name.len() as u64)
            .sum();
        entries + central + TAIL_LEN
    }

    /// Where the local header of entry `index` starts.
    fn offset(&self, index: usize) -> u64 {
        self.entries[..index]
            .iter()
            .map(|e| LOCAL_FIXED + e.name.len() as u64 + e.size + DESCRIPTOR_LEN)
            .sum()
    }

    /// The local header of entry `index`, which comes before its data.
    pub(crate) fn local_header(&self, index: usize) -> Vec<u8> {
        let entry = &self.entries[index];
        let mut out = Vec::with_capacity(LOCAL_FIXED as usize + entry.name.len());
        u32le(&mut out, LOCAL_HEADER);
        u16le(&mut out, VERSION);
        u16le(&mut out, FLAGS);
        u16le(&mut out, 0); // stored
        u16le(&mut out, entry.time);
        u16le(&mut out, entry.date);
        u32le(&mut out, 0); // checksum: in the data descriptor
        u32le(&mut out, u32::MAX);
        u32le(&mut out, u32::MAX);
        u16le(&mut out, entry.name.len() as u16);
        u16le(&mut out, 20);
        out.extend_from_slice(&entry.name);
        // The zip64 sizes: in the data descriptor.
        u16le(&mut out, 1);
        u16le(&mut out, 16);
        u64le(&mut out, 0);
        u64le(&mut out, 0);
        out
    }

    /// The data descriptor that follows the data of entry `index`.
    pub(crate) fn descriptor(&self, index: usize, crc: u32) -> Vec<u8> {
        let size = self.entries[index].size;
        let mut out = Vec::with_capacity(DESCRIPTOR_LEN as usize);
        u32le(&mut out, DATA_DESCRIPTOR);
        u32le(&mut out, crc);
        u64le(&mut out, size);
        u64le(&mut out, size);
        out
    }

    /// The central directory and end records, given every entry's checksum.
    pub(crate) fn central_directory(&self, crcs: &[u32]) -> Vec<u8> {
        let start = self.offset(self.entries.len());
        let mut out = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            u32le(&mut out, CENTRAL_HEADER);
            u16le(&mut out, (3 << 8) | VERSION); // made by Unix
            u16le(&mut out, VERSION);
            u16le(&mut out, FLAGS);
            u16le(&mut out, 0);
            u16le(&mut out, entry.time);
            u16le(&mut out, entry.date);
            u32le(&mut out, crcs[index]);
            u32le(&mut out, u32::MAX);
            u32le(&mut out, u32::MAX);
            u16le(&mut out, entry.name.len() as u16);
            u16le(&mut out, 28);
            u16le(&mut out, 0); // comment
            u16le(&mut out, 0); // disk
            u16le(&mut out, 0); // internal attributes
            u32le(&mut out, 0o100644 << 16); // a regular file, rw-r--r--
            u32le(&mut out, u32::MAX);
            out.extend_from_slice(&entry.name);
            u16le(&mut out, 1);
            u16le(&mut out, 24);
            u64le(&mut out, entry.size);
            u64le(&mut out, entry.size);
            u64le(&mut out, self.offset(index));
        }
        let central_size = out.len() as u64;
        let count = self.entries.len() as u64;
        let zip64_end = start + central_size;

        u32le(&mut out, ZIP64_END);
        u64le(&mut out, 44);
        u16le(&mut out, (3 << 8) | VERSION);
        u16le(&mut out, VERSION);
        u32le(&mut out, 0);
        u32le(&mut out, 0);
        u64le(&mut out, count);
        u64le(&mut out, count);
        u64le(&mut out, central_size);
        u64le(&mut out, start);

        u32le(&mut out, ZIP64_LOCATOR);
        u32le(&mut out, 0);
        u64le(&mut out, zip64_end);
        u32le(&mut out, 1);

        u32le(&mut out, END);
        u16le(&mut out, 0);
        u16le(&mut out, 0);
        u16le(&mut out, u16::MAX);
        u16le(&mut out, u16::MAX);
        u32le(&mut out, u32::MAX);
        u32le(&mut out, u32::MAX);
        u16le(&mut out, 0);
        out
    }
}

/// 1980-01-01, the earliest date a zip can hold.
const DOS_EPOCH_DATE: u16 = 0x21;

/// The DOS date and time of `millis` since the Unix epoch, in UTC.
fn dos_date_time(millis: i64) -> (u16, u16) {
    let seconds = millis.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    if !(1980..=2107).contains(&year) {
        return (DOS_EPOCH_DATE, 0);
    }
    let date = (((year - 1980) as u16) << 9) | ((month as u16) << 5) | day as u16;
    let time = (((of_day / 3600) as u16) << 11)
        | ((((of_day % 3600) / 60) as u16) << 5)
        | ((of_day % 60) / 2) as u16;
    (date, time)
}

/// The proleptic Gregorian date of `days` since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn u16le(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn u32le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn u64le(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, size: u64) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            size,
            modified: Some(1_700_000_000_000),
        }
    }

    /// Writes the zip of `files` the way a response streams it.
    fn write_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let entries: Vec<_> = files
            .iter()
            .map(|(name, data)| entry(name, data.len() as u64))
            .collect();
        let layout = ZipLayout::new(&entries);
        let mut out = Vec::new();
        let mut crcs = Vec::new();
        for (index, (_, data)) in files.iter().enumerate() {
            out.extend(layout.local_header(index));
            out.extend_from_slice(data);
            let crc = crc32fast::hash(data);
            crcs.push(crc);
            out.extend(layout.descriptor(index, crc));
        }
        out.extend(layout.central_directory(&crcs));
        assert_eq!(out.len() as u64, layout.total_size());
        out
    }

    #[test]
    fn dates_convert_to_dos_form() {
        // 2023-11-14 22:13:20 UTC.
        let (date, time) = dos_date_time(1_700_000_000_000);
        assert_eq!(date, ((2023 - 1980) << 9) | (11 << 5) | 14);
        assert_eq!(time, (22 << 11) | (13 << 5) | 10);
        assert_eq!(dos_date_time(0), (DOS_EPOCH_DATE, 0));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn the_archive_has_the_announced_size_and_structure() {
        let zip = write_zip(&[
            ("run/a.txt", b"alpha"),
            ("run/empty", b""),
            ("b.bin", b"\x00\x01"),
        ]);
        assert_eq!(&zip[..4], &LOCAL_HEADER.to_le_bytes());
        let end = zip.len() - 22;
        assert_eq!(&zip[end..end + 4], &END.to_le_bytes());
        let locator = end - 20;
        assert_eq!(&zip[locator..locator + 4], &ZIP64_LOCATOR.to_le_bytes());
        let zip64_end = u64::from_le_bytes(zip[locator + 8..locator + 16].try_into().unwrap());
        assert_eq!(
            &zip[zip64_end as usize..zip64_end as usize + 4],
            &ZIP64_END.to_le_bytes()
        );
    }

    /// Reads the archive back with Python's `zipfile`, an independent reader,
    /// when an interpreter is available.
    #[test]
    fn a_reference_reader_extracts_the_archive() {
        let files: [(&str, &[u8]); 3] = [
            ("run/a.txt", b"alpha"),
            ("run/sub/empty", b""),
            ("ünï.bin", &[7; 1000]),
        ];
        let zip = write_zip(&files);
        let path = std::env::temp_dir().join(format!("egui_states_zip_{}.zip", std::process::id()));
        std::fs::write(&path, &zip).unwrap();
        // Written as UTF-8 bytes: `print` would use the console's encoding
        // and line ends, which on Windows are neither UTF-8 nor `\n`.
        let script = "import sys, zipfile\n\
            z = zipfile.ZipFile(sys.argv[1])\n\
            assert z.testzip() is None\n\
            for i in z.infolist():\n    \
            line = f'{i.filename} {len(z.read(i))} {i.date_time[0]}\\n'\n    \
            sys.stdout.buffer.write(line.encode('utf-8'))\n";
        let output = std::process::Command::new("python3")
            .args(["-c", script])
            .arg(&path)
            .output();
        let _ = std::fs::remove_file(&path);
        let Ok(output) = output else {
            eprintln!("python3 not found; the reference check is skipped");
            return;
        };
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "run/a.txt 5 2023\nrun/sub/empty 0 2023\nünï.bin 1000 2023\n"
        );
    }
}
