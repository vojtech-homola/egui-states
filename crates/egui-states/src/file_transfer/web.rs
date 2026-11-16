//! Browser files: the source of uploads from the web client.

use web_sys::js_sys::Uint8Array;
use web_sys::wasm_bindgen::JsCast;

/// Bytes read from a browser file at once.
const BLOCK: f64 = 1024.0 * 1024.0;

/// A browser `File`.
#[derive(Clone, Debug)]
pub(crate) struct WebFile(pub(crate) web_sys::File);

// SAFETY: the WebAssembly client runs on one thread; a `File` never reaches
// another one. The engine's shared state needs `Send`, which native files
// have.
unsafe impl Send for WebFile {}
// SAFETY: as above.
unsafe impl Sync for WebFile {}

/// The files of an upload, read one slice after another.
pub(crate) struct WebSource {
    files: Vec<(WebFile, u64)>,
    next: usize,
    offset: u64,
    buffer: Vec<u8>,
    position: usize,
}

impl WebSource {
    pub(crate) fn new(files: Vec<(WebFile, u64)>) -> Self {
        Self {
            files,
            next: 0,
            offset: 0,
            buffer: Vec::new(),
            position: 0,
        }
    }

    pub(crate) async fn read(&mut self, len: usize) -> Result<Vec<u8>, String> {
        self.buffer.drain(..self.position);
        self.position = 0;
        let want = (len as f64).max(BLOCK) as usize;
        while self.buffer.len() < len {
            let Some((file, size)) = self.files.get(self.next) else {
                break;
            };
            if file.0.size() as u64 != *size {
                return Err(format!("{} changed size after the offer", file.0.name()));
            }
            let end = (self.offset + (want - self.buffer.len()) as u64).min(*size);
            let blob = file
                .0
                .slice_with_f64_and_f64(self.offset as f64, end as f64)
                .map_err(|error| format!("cannot read {}: {error:?}", file.0.name()))?;
            let buffer = wasm_bindgen_futures::JsFuture::from(blob.array_buffer())
                .await
                .map_err(|error| format!("cannot read {}: {error:?}", file.0.name()))?;
            let bytes = Uint8Array::new(&buffer.unchecked_into());
            let start = self.buffer.len();
            self.buffer.resize(start + bytes.length() as usize, 0);
            bytes.copy_to(&mut self.buffer[start..]);
            self.offset = end;
            if self.offset == *size {
                self.next += 1;
                self.offset = 0;
            }
        }
        if self.buffer.len() < len {
            return Err("the files are shorter than announced".to_string());
        }
        self.position = len;
        Ok(self.buffer[..len].to_vec())
    }
}
