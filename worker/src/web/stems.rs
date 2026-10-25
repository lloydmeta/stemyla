//! The stem writer: writes the stems of one separation to their session
//! files as the segments become ready, so the stems never cross to the
//! page (the storage path, M3 of `plans/active/stream-from-storage.md`).
//!
//! It writes each stem to `<session>/stems/<file>.part`, where `<file>` is
//! [`stem_protocol::stem_file`] of its name.  The page moves each file onto
//! its `.wav` name after the separation, so the kept stems of an earlier
//! separation stay until the new ones are complete.  A file is a 16-bit
//! stereo WAV at 44.1 kHz, the layout of `engine::wav::header`, which the
//! worker cannot take (LAYER-1).  The header holds sizes of 0 until
//! [`StemWriter::finish`] writes the real ones.
//!
//! Each file is written through a `FileSystemSyncAccessHandle`, which only
//! a dedicated worker has.  Each write gets a whole new `Uint8Array`, never
//! a view (MEM-5).

use js_sys::{Array, Uint8Array};
use roformer::SAMPLE_RATE;
use roformer::demix::Segment;
use wasm_bindgen::prelude::*;

use super::{js_text, pcm16};

#[wasm_bindgen(inline_js = r#"
async function session_folder(path) {
  let d = await navigator.storage.getDirectory();
  for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p);
  return d;
}
// `write` returns the bytes written.  Old browsers return a promise, so
// it is awaited, as `truncate`, `flush` and `close` are.
async function put(h, bytes, at) {
  const n = await h.write(bytes, { at });
  if (n !== bytes.byteLength) throw new Error(`wrote ${n} of ${bytes.byteLength} bytes`);
}
// The session folder must exist: it is never made here, so a forgotten
// session does not come back.
// `ours`: the files that this writer made or holds.  A file that another
// writer holds is not ours, so a failed open never removes it.
export async function stems_open(session, files, header) {
  const dir = await (await session_folder(session)).getDirectoryHandle("stems", { create: true });
  const w = { dir, handles: [], ours: [] };
  try {
    for (const name of files) {
      let fh;
      try { fh = await dir.getFileHandle(name); }
      catch (e) {
        if (e.name !== "NotFoundError") throw e;
        fh = await dir.getFileHandle(name, { create: true });
        w.ours.push(name);
      }
      const h = await fh.createSyncAccessHandle();
      w.handles.push(h);
      if (!w.ours.includes(name)) w.ours.push(name);
      await h.truncate(0);
      await put(h, header, 0);
    }
  } catch (e) {
    await stems_abandon(w);
    throw e;
  }
  return w;
}
export async function stems_write(w, i, bytes, at) {
  await put(w.handles[i], bytes, at);
}
export async function stems_finish(w, header) {
  for (const h of w.handles) {
    await put(h, header, 0);
    await h.flush();
  }
  for (const h of w.handles) await h.close();
}
// Closes each handle and removes each file that this writer made or held.
export async function stems_abandon(w) {
  for (const h of w.handles) {
    try { await h.close(); } catch (_) {}
  }
  for (const name of w.ours) {
    try { await w.dir.removeEntry(name); } catch (_) {}
  }
}
"#)]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn stems_open(
        session: &str,
        files: &Array,
        header: &Uint8Array,
    ) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn stems_write(w: &JsValue, i: u32, bytes: &Uint8Array, at: f64) -> Result<(), JsValue>;
    #[wasm_bindgen(catch)]
    async fn stems_finish(w: &JsValue, header: &Uint8Array) -> Result<(), JsValue>;
    async fn stems_abandon(w: &JsValue);
}

const HEADER_BYTES: usize = 44;
const FRAME_BYTES: usize = 4;

/// The most frames a WAV file holds: its RIFF size, `36 + 4 x frames`, is
/// a `u32`.
pub(super) const MAX_WAV_FRAMES: usize = (u32::MAX as usize - 36) / FRAME_BYTES;

/// Why the stems could not be written.
#[derive(Debug)]
pub(super) enum StemError {
    /// Browser storage refused: the browser's error name and text, such as
    /// "QuotaExceededError: ...".
    Storage(String),
    /// The separation cannot be written as asked: a song too long for a
    /// WAV, or segments that do not follow on.
    Refused(String),
}

/// One sync access handle for each stem file of one separation.
pub(super) struct StemWriter {
    js: JsValue,
    stems: usize,
    /// Frames of the song.
    len: usize,
    /// Frames of each stem in its file so far.
    frames: usize,
}

/// The header of a 16-bit stereo WAV of `frames` frames, as
/// `engine::wav::header` writes it.
fn header(frames: usize) -> Uint8Array {
    let data = (frames * FRAME_BYTES) as u32;
    let mut out = Vec::with_capacity(HEADER_BYTES);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * FRAME_BYTES as u32).to_le_bytes());
    out.extend_from_slice(&(FRAME_BYTES as u16).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    Uint8Array::from(out.as_slice())
}

fn storage(e: JsValue) -> StemError {
    StemError::Storage(js_text(&e))
}

impl StemWriter {
    /// Opens the `.part` file of each stem in `names`, in the folder
    /// `session` (`"sessions/<id>"`), for a song of `len` frames.  It
    /// empties each file, and writes a header with sizes of 0.  When it
    /// fails, it removes the files that it made or held, but never a file
    /// that another writer holds.
    pub(super) async fn open(session: &str, names: &[&str], len: usize) -> Result<Self, StemError> {
        if len > MAX_WAV_FRAMES {
            return Err(StemError::Refused(format!(
                "the song is too long to store: {len} frames, and a WAV file holds {MAX_WAV_FRAMES}"
            )));
        }
        let files: Array = names
            .iter()
            .map(|n| JsValue::from(format!("{}.part", stem_protocol::stem_file(n))))
            .collect();
        let js = stems_open(session, &files, &header(0))
            .await
            .map_err(storage)?;
        Ok(Self {
            js,
            stems: names.len(),
            len,
            frames: 0,
        })
    }

    /// Writes each stem's samples of `seg`, which must follow on from the
    /// last segment, as interleaved 16-bit samples.
    pub(super) async fn put(&mut self, seg: &Segment) -> Result<(), StemError> {
        let refused = |text: String| Err(StemError::Refused(text));
        if seg.stems.len() != self.stems {
            return refused(format!(
                "a segment has {} stems, not {}",
                seg.stems.len(),
                self.stems
            ));
        }
        if seg.start != self.frames {
            return refused(format!(
                "a segment starts at frame {}, not {}",
                seg.start, self.frames
            ));
        }
        let n = seg.stems.first().map_or(0, |[l, _]| l.len());
        if seg.stems.iter().any(|[l, r]| l.len() != n || r.len() != n) {
            return refused("the stems of a segment differ in length".into());
        }
        if self.frames + n > self.len {
            return refused(format!(
                "a segment ends at frame {}, after the song's {}",
                self.frames + n,
                self.len
            ));
        }
        let at = (HEADER_BYTES + FRAME_BYTES * seg.start) as f64;
        let mut bytes = vec![0u8; FRAME_BYTES * n];
        for (i, [left, right]) in seg.stems.iter().enumerate() {
            for ((frame, l), r) in bytes.chunks_exact_mut(FRAME_BYTES).zip(left).zip(right) {
                frame[..2].copy_from_slice(&(pcm16(*l) as i16).to_le_bytes());
                frame[2..].copy_from_slice(&(pcm16(*r) as i16).to_le_bytes());
            }
            // A new array of its own, never a view of WebAssembly memory.
            let whole = Uint8Array::from(bytes.as_slice());
            stems_write(&self.js, i as u32, &whole, at)
                .await
                .map_err(storage)?;
        }
        self.frames += n;
        Ok(())
    }

    /// Frames of each stem in its file so far.
    pub(super) fn frames(&self) -> usize {
        self.frames
    }

    /// Writes the header with the song's sizes, then flushes and closes
    /// each file.  It fails if a stem holds fewer frames than the song.
    pub(super) async fn finish(&mut self) -> Result<(), StemError> {
        if self.frames != self.len {
            return Err(StemError::Refused(format!(
                "the stems hold {} of the song's {} frames",
                self.frames, self.len
            )));
        }
        stems_finish(&self.js, &header(self.len))
            .await
            .map_err(storage)
    }

    /// Closes each file and removes it.  It removes nothing else.
    pub(super) async fn abandon(self) {
        stems_abandon(&self.js).await;
    }
}
