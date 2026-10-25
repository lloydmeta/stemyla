//! The stored mix: a mix file in browser storage that the worker reads
//! one span at a time, so its memory does not grow with the song (M4 of
//! `plans/active/stream-from-storage.md`).
//!
//! The file holds interleaved 32-bit little-endian floats, two for each
//! frame, with no header.  The worker opens it once with `getFile()`,
//! which takes no lock.  The page does not write the file while a
//! separation runs.  Each span is read with `FileReaderSync`, so the
//! demixer's loop needs no `await` to read.

use roformer::demix::{MixSource, Stereo};
use wasm_bindgen::prelude::*;
use web_sys::Blob;

use super::{blob_range, js_text};

#[wasm_bindgen(inline_js = r#"
export async function mix_file(path) {
  const parts = path.split("/").filter((s) => s);
  const name = parts.pop();
  let d = await navigator.storage.getDirectory();
  for (const p of parts) d = await d.getDirectoryHandle(p);
  return await (await d.getFileHandle(name)).getFile();
}
"#)]
extern "C" {
    /// The file at `path` from the root of the private file system, as a
    /// `File` of its state on disk now.
    #[wasm_bindgen(catch)]
    async fn mix_file(path: &str) -> Result<Blob, JsValue>;
}

/// Bytes in one frame of the mix file: two 32-bit floats.
const FRAME_BYTES: usize = 8;

/// A mix file, opened and checked against its frames.
pub(super) struct StoredMix {
    file: Blob,
    frames: usize,
}

impl StoredMix {
    /// Opens the mix file at `path`, and checks that it holds `frames`
    /// frames.  The error is a text for `unstored`.
    pub(super) async fn open(path: &str, frames: usize) -> Result<Self, String> {
        let file = mix_file(path)
            .await
            .map_err(|e| format!("the mix file could not be opened: {}", js_text(&e)))?;
        // In f64: 8 x frames of a long song does not fit a wasm32 usize.
        let want = FRAME_BYTES as f64 * frames as f64;
        if file.size() != want {
            return Err(format!(
                "the mix file holds {} bytes, not the {want} bytes of {frames} frames",
                file.size()
            ));
        }
        Ok(Self { file, frames })
    }
}

impl MixSource for StoredMix {
    type Error = String;

    fn frames(&self) -> usize {
        self.frames
    }

    fn read(&mut self, start: usize, end: usize) -> Result<Stereo, String> {
        let at = |frame: usize| FRAME_BYTES as f64 * frame as f64;
        let bytes = blob_range(&self.file, at(start), at(end))
            .map_err(|e| format!("the mix file could not be read: {}", js_text(&e)))?;
        let want = FRAME_BYTES * (end - start);
        if bytes.length() as usize != want {
            return Err(format!(
                "the mix file gave {} bytes of frames {start} to {end}, not {want}",
                bytes.length()
            ));
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1: one span of the mix file, at most one chunk, not the song"
        )]
        let bytes = bytes.to_vec();
        let sample =
            |k: usize| f32::from_le_bytes([bytes[k], bytes[k + 1], bytes[k + 2], bytes[k + 3]]);
        let channel = |c: usize| {
            (0..end - start)
                .map(|f| sample(FRAME_BYTES * f + 4 * c))
                .collect()
        };
        Ok([channel(0), channel(1)])
    }
}
