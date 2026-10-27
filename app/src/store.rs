//! Song sessions kept in the browser's private file system (OPFS), so a
//! reload brings back the song and its separated stems without running the
//! network again.  Web only.
//!
//! Layout under the OPFS root:
//! * `sessions/current`: the id of the session shown last (text).
//! * `sessions/<id>/meta`: tab-separated lines: `name`, `rate`, one `file`
//!   line per source file (original name, stored name), one `stem` line
//!   per separated stem, a `mix` line (frames, format) when `mix.f32` is
//!   complete, and a `decoded` line (frames, format, the channels of each
//!   source) when the decode worker's files are complete (D2 of M5).  A
//!   `stems-generation` line counts the changes of the `stem` lines, and a
//!   `stem-overviews` line (frames, format, generation) names complete
//!   overviews of the stems of that generation (D7 of M7).
//! * `sessions/<id>/decoded/<i>.f32`: source `<i>` decoded at 44.1 kHz, as
//!   `mix.f32` is, padded to the song's frames.  The source in the mix of a
//!   song of one source has no file here: `mix.f32` is that source (D1 of
//!   M5).
//! * `sessions/<id>/overviews/<i>.f32` and `overviews/mix.f32`: the
//!   waveform columns of each source lane and of the mix lane, each column
//!   its minimum and its maximum as 32-bit little-endian floats.
//! * `sessions/<id>/overviews/stems/<k>.f32` and `overviews/stems/mix.f32`:
//!   the waveform columns of the stem of the `k`th `stem` line and of the
//!   stems' mix, in the same layout ([`stem_overviews`], D7 of M7).
//! * `sessions/<id>/classic.wav` and `overviews/classic.f32`: classic drums
//!   of the song as a 16-bit WAV, and its overview ([`keep_classic`], D8 of
//!   M7).  `meta` does not name them: they are scratch files.
//! * `<file>.decode.part`: a file of the list above that the decode worker
//!   wrote.  [`keep_decoded`] moves it onto its name (D7 of M5).  An
//!   overview of the stems is `<file>.<generation>.decode.part`.
//! * `sessions/<id>/src/<n>.<ext>`: the files as dropped.
//! * `sessions/<id>/mix.f32`: the mix that the worker separates on the
//!   storage path (M4): raw 32-bit little-endian floats, interleaved
//!   stereo, no header, at 44.1 kHz.  [`write_mix`] writes it in blocks
//!   through `mix.f32.part`, and only then writes the `mix` line.  The
//!   page uses it only when the `mix` line has the song's frames and
//!   [`MIX_FORMAT`], and the file's size is 8 bytes for each frame.
//! * `sessions/<id>/stems/<stem>.wav`: separated stems, 16-bit PCM.  The
//!   file name is `stem_protocol::stem_file` of the stem's name.
//! * `sessions/<id>/stems/<stem>.wav.part`: a stem that the worker writes
//!   as it separates (the storage path of M3).  After the separation,
//!   [`keep_stored_stems`] moves it onto its `.wav` file.  A separation
//!   that stopped can leave one; no reader opens it.
//!
//! To open a song, the page saves the chosen files, and the decode worker
//! decodes them into the session files in blocks (M5).  The page moves the
//! files into place and writes `meta` ([`keep_decoded`]), then shows the
//! song as stored lanes: the session files with their overviews, of which
//! it reads only the overviews ([`stored_sources`], [`stem_lanes`], D11 of
//! M7).  A session whose files are complete opens with no decode
//! ([`open_stored`]).  When browser storage fails, the page decodes the
//! sources itself ([`restore_id`]), and the stems are stored lanes all the
//! same.
//!
//! On the storage path, the page writes the mix file before it posts, the
//! worker reads it and writes the stem files, and the page moves them into
//! place and writes `meta`.  On the memory path, the page
//! writes the stems itself ([`save_stems`]).
//!
//! Since M6, the shown song plays each lane that has a complete session
//! file from that file (D8 of `plans/active/stream-from-storage-m6.md`);
//! since M7, such a lane has no samples in memory.  [`read_stem`] reads a
//! stem whole only when its file is not complete, on the memory path's
//! rules.  While
//! the shown song plays from a session ([`set_playing`]), or an open reads
//! it ([`OpenGuard`]), the session is [`protected`]: [`forget_mix`] removes
//! nothing until it is not (D10).
//!
//! Ids are creation times in milliseconds, made unique within the page: a
//! second session in the same millisecond takes the next one.  Every
//! dropped song gets its own session; the Storage menu lists them, opens
//! one or forgets it.

use crate::store_meta::{Decoded, LaneFile, MIX_FORMAT, Meta, PlannedSource};
use anyhow::{Result, anyhow};
use engine::song::padded_to;
use engine::{Samples, Song, Stem};
use js_sys::{Array, Uint8Array};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use stem_protocol::stem_file;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
async function dir(path, create, from) {
  let d = from || await navigator.storage.getDirectory();
  for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p, { create });
  return d;
}
function split(path) {
  const i = path.lastIndexOf("/");
  return i < 0 ? ["", path] : [path.slice(0, i), path.slice(i + 1)];
}
// Safari 27 writes a typed array view's whole buffer, not just the view
// (`new Uint8Array(16).subarray(4, 8)` writes 16 bytes), so only whole
// arrays are written: a view is copied out first.
function whole(bytes) {
  return bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength ? bytes : bytes.slice();
}
export async function st_write(within, rel, body) {
  let w;
  try {
    let base;
    try { base = await dir(within, false); }
    catch (e) { if (within && e.name === "NotFoundError") return "gone"; throw e; }
    const [d, n] = split("/" + rel);
    const fh = await (await dir(d, true, base)).getFileHandle(n, { create: true });
    w = await fh.createWritable();
    await w.write(body instanceof Blob ? body : whole(body));
    await w.close();
    return true;
  } catch (e) {
    if (w) try { await w.abort(); } catch (_) {}
    console.warn("storage write failed", within, rel, e);
    return (e && e.name) || "Error";
  }
}
export async function st_read(path) {
  try {
    const [d, n] = split(path);
    const f = await (await (await dir(d, false)).getFileHandle(n)).getFile();
    return new Uint8Array(await f.arrayBuffer());
  } catch (e) { return null; }
}
export async function st_remove(path) {
  try {
    const [d, n] = split(path);
    await (await dir(d, false)).removeEntry(n, { recursive: true });
    return true;
  } catch (e) { return e.name === "NotFoundError"; }
}
export async function st_move(path, from, to) {
  try {
    const d = await dir(path, false);
    await (await d.getFileHandle(from)).move(d, to);
    return true;
  } catch (e) {
    console.warn("storage move failed", path, from, to, e);
    return false;
  }
}
export async function st_exists(path) {
  try { await dir(path, false); return true; }
  catch (e) { return e.name !== "NotFoundError"; }
}
export async function st_list(path) {
  try {
    const out = [];
    for await (const [name, h] of (await dir(path, false)).entries()) if (h.kind === "directory") out.push(name);
    return out;
  } catch (e) { return []; }
}
export async function st_usage() {
  try {
    const e = await navigator.storage.estimate();
    return [e.usage || 0, e.quota || 0];
  } catch (e) { return [0, 0]; }
}
export function save_file(name, bytes, mime) {
  save_blob(name, new Blob([bytes], { type: mime }));
}
export function save_blob(name, blob) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60000);
}
export function st_sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
// A writer of `within/rel`, as `st_write` opens it; else "gone" or the
// name of the error.
export async function st_open(within, rel) {
  try {
    let base;
    try { base = await dir(within, false); }
    catch (e) { if (within && e.name === "NotFoundError") return "gone"; throw e; }
    const [d, n] = split("/" + rel);
    const fh = await (await dir(d, true, base)).getFileHandle(n, { create: true });
    return await fh.createWritable();
  } catch (e) {
    console.warn("storage open failed", within, rel, e);
    return (e && e.name) || "Error";
  }
}
// Writes `bytes` at the writer's position: true, else the error's name.
export async function st_put(w, bytes) {
  try { await w.write(whole(bytes)); return true; }
  catch (e) { console.warn("storage write failed", e); return (e && e.name) || "Error"; }
}
export async function st_close(w) {
  try { await w.close(); return true; }
  catch (e) { console.warn("storage close failed", e); return (e && e.name) || "Error"; }
}
export async function st_abort(w) {
  try { await w.abort(); } catch (_) {}
}
export async function st_size(path) {
  try {
    const [d, n] = split(path);
    return (await (await (await dir(d, false)).getFileHandle(n)).getFile()).size;
  } catch (e) { return null; }
}
// The file at `path` as a `File`, which holds no bytes until it is read,
// or null if there is none.
export async function st_file(path) {
  try {
    const [d, n] = split(path);
    return await (await (await dir(d, false)).getFileHandle(n)).getFile();
  } catch (e) { return null; }
}
// Bytes `start..end` of a `File`: one read of a slice, never the file.
export async function st_slice(file, start, end) {
  return new Uint8Array(await file.slice(start, end).arrayBuffer());
}
"#)]
extern "C" {
    /// Writes `body`, a `Blob` or a typed array (no other type), to
    /// `within/rel`, creating the folders in `rel`; `within` must exist
    /// already.  True; "gone" when `within` does not exist; else the name
    /// of the error, such as "QuotaExceededError".
    async fn st_write(within: &str, rel: &str, body: &JsValue) -> JsValue;
    async fn st_read(path: &str) -> JsValue;
    /// Whether `path` is gone (removed, or never there).
    async fn st_remove(path: &str) -> JsValue;
    /// Moves the file `from` onto `to` in the folder `path`, replacing
    /// `to`; whether it moved.
    async fn st_move(path: &str, from: &str, to: &str) -> JsValue;
    async fn st_usage() -> JsValue;
    async fn st_list(path: &str) -> JsValue;
    /// Whether the folder `path` exists (true also when the browser cannot
    /// tell).
    async fn st_exists(path: &str) -> JsValue;
    fn save_file(name: &str, bytes: &Uint8Array, mime: &str);
    fn save_blob(name: &str, blob: &web_sys::Blob);
    async fn st_sleep(ms: u32) -> JsValue;
    /// A writable stream of `within/rel`, creating the folders in `rel`;
    /// `within` must exist.  Else "gone" when `within` does not exist, or
    /// the name of the error.
    async fn st_open(within: &str, rel: &str) -> JsValue;
    /// Writes a typed array to the stream `w`: true, else the error's name.
    async fn st_put(w: &JsValue, bytes: &Uint8Array) -> JsValue;
    /// Closes the stream `w`, which keeps what it wrote: true, else the
    /// error's name.
    async fn st_close(w: &JsValue) -> JsValue;
    /// Closes the stream `w` and drops what it wrote.
    async fn st_abort(w: &JsValue);
    /// The bytes in the file at `path`, or null if it cannot be read.
    async fn st_size(path: &str) -> JsValue;
    /// The file at `path` as a `File`, or null if there is none.
    async fn st_file(path: &str) -> JsValue;
    /// Bytes `start..end` of the `File` `file`.
    #[wasm_bindgen(catch)]
    async fn st_slice(file: &JsValue, start: f64, end: f64) -> Result<JsValue, JsValue>;
}

const ROOT: &str = "sessions";

thread_local! {
    /// Writes in progress, each with a file open for writing.
    static WRITES: Cell<usize> = const { Cell::new(0) };
    /// Set by [`close`]: every later write is refused.
    static CLOSED: Cell<bool> = const { Cell::new(false) };
    /// Sessions with stems being saved, which [`restore_id`] and [`forget`]
    /// wait for: a restore would read the meta from before the stems, and
    /// a removal fails while a file in the session is open for writing.
    /// Saves in progress per session (a quick second separation can
    /// overlap the first one's save).
    static SAVING: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
    /// The last session id made, so the next one is later.
    static LAST_ID: Cell<u64> = const { Cell::new(0) };
    /// Decodes in progress per session (see [`DecodeGuard`]).
    static DECODING: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
    /// Mix writes in progress per session ([`write_mix`]).
    static MIX_WRITING: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
    /// The session of the separation that may read or write its mix.
    static SEPARATING: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Sessions whose `meta` a read-modify-write holds ([`lock_meta`]).
    static META_HELD: RefCell<std::collections::HashSet<String>> = RefCell::new(std::collections::HashSet::new());
    /// The session that the shown song plays a lane from (D10 of M6).
    static PLAYING: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Opens in progress per session, each with an [`OpenGuard`].
    static OPENING: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
    /// Sessions whose mix removal waits until no playback, open,
    /// separation or mix write needs the mix ([`forget_mix`]).
    static DEFERRED: RefCell<std::collections::HashSet<String>> = RefCell::new(std::collections::HashSet::new());
}

/// A new session id: now in milliseconds, or one more than the last id if
/// that is later, and never the id of a stored session.  So a save never
/// shares a folder, and a failed save removes only its own.  (Two tabs that
/// pick an id at the same moment can still meet.)
async fn new_id() -> String {
    let mut id = (js_sys::Date::now() as u64).max(LAST_ID.get() + 1);
    while st_exists(&format!("{ROOT}/{id}")).await.as_bool() != Some(false) {
        id += 1;
    }
    LAST_ID.set(id);
    id.to_string()
}

/// The `meta` of a session, held for one read-modify-write, so two writers
/// never interleave and lose each other's lines (finding 1-5 of M5).
/// It is not a save: `settled` does not wait for it, so D8a still holds.
struct MetaLock(String);

/// Waits until no other writer holds the `meta` of session `id`, then
/// holds it until the lock is dropped.
async fn lock_meta(id: &str) -> MetaLock {
    while !META_HELD.with_borrow_mut(|held| held.insert(id.to_string())) {
        st_sleep(10).await;
    }
    MetaLock(id.to_string())
}

impl Drop for MetaLock {
    fn drop(&mut self) {
        META_HELD.with_borrow_mut(|held| held.remove(&self.0));
    }
}

async fn settled(id: &str) {
    while SAVING.with_borrow(|s| s.contains_key(id)) {
        st_sleep(50).await;
    }
}

/// Refuses every later write and waits for those in progress, so a reset
/// can delete the stored files without a writer holding one open or a
/// save recreating one.  Storage stays closed until the page reloads.
pub async fn close() {
    CLOSED.set(true);
    while WRITES.get() > 0 {
        st_sleep(50).await;
    }
}

async fn write(path: &str, bytes: &[u8]) -> Result<()> {
    write_bytes("", path, bytes).await
}

async fn write_bytes(within: &str, rel: &str, bytes: &[u8]) -> Result<()> {
    write_within(within, rel, &Uint8Array::from(bytes)).await
}

/// Writes `body`, a `Blob` or a typed array (no other type), to
/// `within/rel` only if the folder `within` still exists, so a save into a
/// session forgotten meanwhile cannot bring it back.
async fn write_within(within: &str, rel: &str, body: &JsValue) -> Result<()> {
    if CLOSED.get() {
        return Err(anyhow!("browser storage is being reset"));
    }
    WRITES.set(WRITES.get() + 1);
    let written = st_write(within, rel, body).await;
    WRITES.set(WRITES.get() - 1);
    match (written.as_bool(), written.as_string().as_deref()) {
        (Some(true), _) => Ok(()),
        (_, Some("gone")) => Err(anyhow!("{within} is gone (forgotten?)")),
        (_, error) => Err(anyhow!(
            "browser storage refused {} ({})",
            if within.is_empty() {
                rel.to_string()
            } else {
                format!("{within}/{rel}")
            },
            error.unwrap_or("not available")
        )),
    }
}

async fn read(path: &str) -> Option<Vec<u8>> {
    let v = st_read(path).await;
    if v.is_null() {
        None
    } else {
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1: a stored file is copied once into wasm memory, for the decoder or to parse it"
        )]
        let bytes = v.unchecked_into::<Uint8Array>().to_vec();
        Some(bytes)
    }
}

/// Starts a session for files the user dropped, and returns its id.  It
/// does not make the session current: the page does that once the song
/// opens (D9, [`make_current`]).  Each file is written as the browser's `Blob`, so the
/// page keeps no copy of its bytes to save it (MEM-1).  A save that fails
/// removes what it wrote: the caller gets no id to use, and a session
/// without `meta` is not listed, so it cannot be forgotten.
pub async fn save_sources(files: &[(String, web_sys::Blob)], rate: u32) -> Result<String> {
    let id = new_id().await;
    let base = format!("{ROOT}/{id}");
    let saved = save_sources_now(&base, files, rate).await;
    if saved.is_err() && st_remove(&base).await.as_bool() != Some(true) {
        log::warn!("could not remove {base} after a failed save");
    }
    saved.map(|()| id)
}

async fn save_sources_now(base: &str, files: &[(String, web_sys::Blob)], rate: u32) -> Result<()> {
    let mut meta = Meta {
        rate,
        ..Default::default()
    };
    for (i, (name, file)) in files.iter().enumerate() {
        let ext = name
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 5 && *e != name)
            .unwrap_or("bin")
            .to_ascii_lowercase();
        let stored = format!("{i}.{ext}");
        write_within("", &format!("{base}/src/{stored}"), file).await?;
        meta.files.push((name.clone(), stored));
    }
    meta.name = files
        .first()
        .map(|(n, _)| crate::store_meta::file_title(n))
        .unwrap_or_default();
    write(&format!("{base}/meta"), meta.to_text().as_bytes()).await
}

/// The folder of session `id`, from the root of the private file system,
/// as the worker's storage path takes it.
pub fn session_folder(id: &str) -> String {
    format!("{ROOT}/{id}")
}

/// The frames that [`write_mix`] writes in one block: 1 MiB of the mix
/// file (D3 of M4).
pub const MIX_BLOCK_FRAMES: usize = 131_072;

const MIX_FILE: &str = "mix.f32";

/// The mix file of session `id`, from the root of the private file system.
pub fn mix_path(id: &str) -> String {
    format!("{}/{MIX_FILE}", session_folder(id))
}

/// The bytes in the file at `path`, or `None` if there is no such file.
async fn file_size(path: &str) -> Option<f64> {
    st_size(path).await.as_f64()
}

/// The frames of session `id`'s usable mix file (D1 of M4): `meta` has a
/// `mix` line in [`MIX_FORMAT`], and the file holds 8 bytes for each of
/// its frames.  It waits for any save into the session first, such as the
/// removal of a mix by [`forget_mix`].
pub async fn stored_mix_frames(id: &str) -> Option<usize> {
    settled(id).await;
    let base = session_folder(id);
    let meta = Meta::parse(&String::from_utf8_lossy(
        &read(&format!("{base}/meta")).await?,
    ));
    let frames = meta.usable_mix()?;
    (file_size(&mix_path(id)).await? == 8.0 * frames as f64).then_some(frames)
}

/// Writes `input`, the memory path's input of the shown song (D9 of the M7
/// plan: its one source lane, or the mix of its sources), to session `id`
/// as its mix file, in blocks of [`MIX_BLOCK_FRAMES`], each a new whole
/// array (D3 of M4, MEM-5).  It reads `input` a block of a lane at a time:
/// lanes in memory from their samples, stored lanes through the lane
/// reader (D10 of the M7 plan).  So the page makes no song-length buffer.
/// It first removes the `mix` line and any old mix file, and it writes the
/// `mix` line only for a complete file.  It never makes a forgotten
/// session folder again.
pub async fn write_mix(id: &str, input: Box<dyn engine::mixdown::Pcm>) -> Result<()> {
    saving(id, async {
        if CLOSED.get() {
            return Err(anyhow!("browser storage is being reset"));
        }
        WRITES.set(WRITES.get() + 1);
        MIX_WRITING.with_borrow_mut(|w| *w.entry(id.to_string()).or_default() += 1);
        let written = write_mix_now(id, input.as_ref()).await;
        MIX_WRITING.with_borrow_mut(|w| {
            if let Some(n) = w.get_mut(id) {
                *n -= 1;
                if *n == 0 {
                    w.remove(id);
                }
            }
        });
        WRITES.set(WRITES.get() - 1);
        written
    })
    .await
}

async fn write_mix_now(id: &str, input: &dyn engine::mixdown::Pcm) -> Result<()> {
    let base = session_folder(id);
    {
        let _meta = lock_meta(id).await;
        let mut meta = meta_now(id).await?;
        if meta.mix.take().is_some() {
            write_bytes(&base, "meta", meta.to_text().as_bytes()).await?;
        }
    }
    if st_remove(&mix_path(id)).await.as_bool() != Some(true) {
        return Err(anyhow!("browser storage could not remove the old mix"));
    }
    let part = format!("{MIX_FILE}.part");
    let w = st_open(&base, &part).await;
    if w.as_string().as_deref() == Some("gone") {
        return Err(anyhow!("{base} is gone (forgotten?)"));
    }
    if let Some(error) = w.as_string() {
        // The open can make the file before the stream fails.
        st_remove(&format!("{base}/{part}")).await;
        return Err(anyhow!("browser storage refused {base}/{part} ({error})"));
    }
    if let Err(e) = put_mix_blocks(&w, input).await {
        st_abort(&w).await;
        st_remove(&format!("{base}/{part}")).await;
        return Err(e);
    }
    let closed = st_close(&w).await;
    if closed.as_bool() != Some(true) || !move_file(&base, &part, MIX_FILE).await {
        st_remove(&format!("{base}/{part}")).await;
        return Err(anyhow!(
            "browser storage could not finish the mix ({})",
            closed.as_string().unwrap_or_else(|| "not moved".into())
        ));
    }
    // `meta` again, under its lock: another writer may have changed it
    // while the mix was written.
    let written = async {
        let _meta = lock_meta(id).await;
        let mut meta = meta_now(id).await?;
        meta.mix = Some((input.frames(), MIX_FORMAT.into()));
        write_bytes(&base, "meta", meta.to_text().as_bytes()).await
    }
    .await;
    if let Err(e) = written {
        st_remove(&mix_path(id)).await;
        return Err(e);
    }
    Ok(())
}

/// Writes the mix of `song` to the stream `w`, one block at a time.
async fn put_mix_blocks(w: &JsValue, input: &dyn engine::mixdown::Pcm) -> Result<()> {
    let frames = input.frames();
    // A read of at most a block of a lane (D3 of the M7 plan).
    let read = (engine::source::BLOCK_FRAMES as usize).min(frames);
    let (mut left, mut right) = (vec![0.0; read], vec![0.0; read]);
    let mut bytes = Vec::with_capacity(8 * MIX_BLOCK_FRAMES.min(frames));
    let mut start = 0;
    while start < frames {
        if CLOSED.get() {
            return Err(anyhow!("browser storage is being reset"));
        }
        let end = (start + MIX_BLOCK_FRAMES).min(frames);
        bytes.clear();
        let mut at = start;
        while at < end {
            let n = read.min(end - at);
            while !input.ready(at, n) {
                st_sleep(1).await;
            }
            input.read(at, &mut left[..n], &mut right[..n]);
            for (l, r) in left[..n].iter().zip(&right[..n]) {
                bytes.extend_from_slice(&l.to_le_bytes());
                bytes.extend_from_slice(&r.to_le_bytes());
            }
            at += n;
        }
        if let Some(e) = input.failed() {
            return Err(anyhow!(
                "the song could not be read from browser storage ({e})"
            ));
        }
        let put = st_put(w, &Uint8Array::from(&bytes[..])).await;
        if put.as_bool() != Some(true) {
            return Err(anyhow!(
                "browser storage refused the mix ({})",
                put.as_string().unwrap_or_default()
            ));
        }
        start = end;
    }
    Ok(())
}

/// Waits `ms` milliseconds.
pub async fn sleep(ms: u32) {
    st_sleep(ms).await;
}

/// Removes the `mix` line of session `id` and its mix file, after the
/// worker could not read it (D6 of M4).  So the next start writes the mix
/// again, and a bad file takes no space.  The removal counts as a save
/// from this call on, before the future first runs, so a start that reads
/// `meta` after this call waits for it ([`stored_mix_frames`]).
///
/// D10 of M6: while the session is [`protected`], or a separation or a
/// mix write names it ([`mix_in_use`]), it removes nothing.  The removal
/// waits, and runs once neither holds.  A separation that posts the mix
/// cancels it ([`keep_mix`]).
pub fn forget_mix(id: &str) -> impl std::future::Future<Output = Result<()>> + 'static {
    let held = SaveHeld::new(id);
    let id = id.to_string();
    async move {
        let _held = held;
        let base = session_folder(&id);
        {
            let _meta = lock_meta(&id).await;
            if protected(&id) || mix_in_use(&id) {
                log::info!(
                    "store: the mix of session {id} is kept while a song plays from it, an open reads it, or a separation or a mix write uses it"
                );
                DEFERRED.with_borrow_mut(|d| d.insert(id.clone()));
                return Ok(());
            }
            DEFERRED.with_borrow_mut(|d| d.remove(&id));
            let mut meta = meta_now(&id).await?;
            if meta.mix.take().is_some() {
                write_bytes(&base, "meta", meta.to_text().as_bytes()).await?;
            }
        }
        if CLOSED.get() || st_remove(&mix_path(&id)).await.as_bool() != Some(true) {
            return Err(anyhow!("browser storage could not remove the mix"));
        }
        Ok(())
    }
}

/// A save into a session, counted in `SAVING` from when it is made until
/// it is dropped.
struct SaveHeld(String);

impl SaveHeld {
    fn new(id: &str) -> Self {
        SAVING.with_borrow_mut(|s| *s.entry(id.to_string()).or_default() += 1);
        Self(id.to_string())
    }
}

impl Drop for SaveHeld {
    fn drop(&mut self) {
        SAVING.with_borrow_mut(|s| {
            if let Some(n) = s.get_mut(&self.0) {
                *n -= 1;
                if *n == 0 {
                    s.remove(&self.0);
                }
            }
        });
    }
}

/// Runs `work` on session `id` as a stem save, which [`restore_id`] and
/// [`forget`] wait for.
async fn saving<T>(id: &str, work: impl std::future::Future<Output = T>) -> T {
    let _held = SaveHeld::new(id);
    work.await
}

/// Adds separated stems to session `id`.
pub async fn save_stems(id: &str, rate: u32, stems: &[Stem]) -> Result<()> {
    saving(id, save_stems_now(id, rate, stems)).await
}

/// What [`keep_stored_stems`] did.
pub struct StoredStems {
    /// Where each stem is to read: its `.wav` file if it moved there, else
    /// its `.part` file.
    pub paths: Vec<String>,
    /// Whether `meta` names the new stems, or why not.
    pub kept: Result<()>,
}

/// After a storage-path separation of session `id`: writes `meta` with no
/// `stem` lines, moves each stem's `.part` file onto its `.wav` file, then
/// writes the new `stem` lines (D5 of M3).  `meta` names the new stems
/// only if every file moved; else it names none.  So a restore never
/// mixes two separations.  This holds also when the page stops part of
/// the way.  If the first write of `meta` fails, nothing moves.
pub async fn keep_stored_stems(id: &str, names: &[String]) -> StoredStems {
    saving(id, keep_stored_now(id, names)).await
}

async fn keep_stored_now(id: &str, names: &[String]) -> StoredStems {
    // Held from the first read of `meta` to its last write; the moves are
    // short.
    let _meta = lock_meta(id).await;
    let base = session_folder(id);
    let folder = format!("{base}/stems");
    let unread = |name: &String| format!("{folder}/{}.part", stem_file(name));
    // Before the first move: if the page stops between the moves and the
    // last write of `meta`, a restore finds no stems, never a mix.
    let mut meta = match read(&format!("{base}/meta")).await {
        Some(bytes) => Meta::parse(&String::from_utf8_lossy(&bytes)),
        None => {
            return StoredStems {
                paths: names.iter().map(unread).collect(),
                kept: Err(anyhow!("session {id} is gone")),
            };
        }
    };
    meta.stems.clear();
    // D7 of the M7 plan: new stems, so the overviews of the old ones go.
    meta.new_stems();
    if let Err(e) = write_bytes(&base, "meta", meta.to_text().as_bytes()).await {
        return StoredStems {
            paths: names.iter().map(unread).collect(),
            kept: Err(e),
        };
    }
    let mut paths = Vec::with_capacity(names.len());
    let mut unmoved = Vec::new();
    for name in names {
        let file = stem_file(name);
        let part = format!("{file}.part");
        if move_file(&folder, &part, &file).await {
            paths.push(format!("{folder}/{file}"));
        } else {
            paths.push(format!("{folder}/{part}"));
            unmoved.push(part);
        }
    }
    let kept = if unmoved.is_empty() {
        #[expect(
            clippy::disallowed_methods,
            reason = "MEM-1: the stem names, a few bytes"
        )]
        let names = names.to_vec();
        meta.stems = names;
        write_bytes(&base, "meta", meta.to_text().as_bytes()).await
    } else {
        Err(anyhow!(
            "browser storage could not move {} into place",
            unmoved.join(", ")
        ))
    };
    StoredStems { paths, kept }
}

/// [`st_move`], refused once storage is closed for a reset.
async fn move_file(folder: &str, from: &str, to: &str) -> bool {
    if CLOSED.get() {
        return false;
    }
    WRITES.set(WRITES.get() + 1);
    let moved = st_move(folder, from, to).await.as_bool() == Some(true);
    WRITES.set(WRITES.get() - 1);
    moved
}

/// The stem `name` from the WAV file at `path`, fitted to a song of
/// `frames`, so the player never reads past a stem's end, with its lane
/// file if the file is complete (D8 of M6).
pub async fn read_stem(path: &str, name: &str, frames: usize) -> Result<(Stem, Option<LaneFile>)> {
    let bytes = read(path)
        .await
        .ok_or_else(|| anyhow!("stem {name} is missing from browser storage"))?;
    let head = &bytes[..bytes.len().min(4_096)];
    let file = crate::store_meta::stem_lane_file(path, head, bytes.len() as u64, frames);
    let audio = engine::decode::decode_bytes(bytes, Some("wav"))?;
    // A session stored before decision 0012 from stems at mixed rates can
    // hold stems one frame shorter than its source now opens with.
    let fitted = |c: Vec<f32>| -> Samples { padded_to(c, frames).into() };
    let mut ch = audio.channels.into_iter();
    let left = fitted(ch.next().unwrap_or_default());
    let right = ch.next().map_or_else(|| left.clone(), fitted);
    let stem = Stem {
        name: name.into(),
        left,
        right,
    };
    Ok((stem, file))
}

async fn save_stems_now(id: &str, rate: u32, stems: &[Stem]) -> Result<()> {
    let base = format!("{ROOT}/{id}");
    // D7 of the M7 plan: before the first stem write, the generation goes
    // up and the overviews of the old stems are no longer named.  `meta`
    // names no stems until the last write, as in `keep_stored_now`, so an
    // `overviews` request meanwhile gives none (finding 6-1 of review round
    // 6 of M7).
    {
        let _meta = lock_meta(id).await;
        let mut meta = meta_now(id).await?;
        meta.stems.clear();
        meta.new_stems();
        write_bytes(&base, "meta", meta.to_text().as_bytes()).await?;
    }
    for s in stems {
        write_bytes(
            &base,
            &format!("stems/{}", stem_file(&s.name)),
            &engine::wav::encode_pcm16(&s.left, &s.right, rate),
        )
        .await?;
    }
    // `meta` again, under its lock: another writer may have changed it
    // while the stems were written.
    let _meta = lock_meta(id).await;
    let mut meta = meta_now(id).await?;
    meta.stems = stems.iter().map(|s| s.name.clone()).collect();
    write_bytes(&base, "meta", meta.to_text().as_bytes()).await
}

pub async fn current() -> Option<String> {
    read(&format!("{ROOT}/current"))
        .await
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Every stored session, newest first.
pub async fn list() -> Vec<(String, Meta)> {
    let names: Array = st_list(ROOT).await.unchecked_into();
    let mut out = Vec::new();
    for name in names.iter().filter_map(|n| n.as_string()) {
        if let Some(bytes) = read(&format!("{ROOT}/{name}/meta")).await {
            out.push((name, Meta::parse(&String::from_utf8_lossy(&bytes))));
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out
}

/// Deletes session `id` (song and stems) once any save into it has
/// finished; false if the browser refused.
pub async fn forget(id: &str) -> bool {
    settled(id).await;
    let removed = st_remove(&format!("{ROOT}/{id}")).await.as_bool() == Some(true);
    if current().await.as_deref() == Some(id) {
        st_remove(&format!("{ROOT}/current")).await;
    }
    removed
}

/// The current session as a song: the dropped files, plus the separated
/// stems and the original mix when there are stems.  `None` when nothing is
/// stored.
pub async fn restore() -> Option<Result<(String, Song, Meta)>> {
    restore_id(&current().await?).await
}

/// Session `id`'s sources as a song, decoded on the page (the memory path).
/// It does not make the session current: the page does, when the song
/// shows (D12 of the M7 plan).
pub async fn restore_id(id: &str) -> Option<Result<(String, Song, Meta)>> {
    settled(id).await;
    let id = id.to_string();
    let base = format!("{ROOT}/{id}");
    let meta = Meta::parse(&String::from_utf8_lossy(
        &read(&format!("{base}/meta")).await?,
    ));
    // D12 of the M7 plan: the page makes the session current only when
    // its song shows, for the newest open.
    let r = rebuild(&base, &meta)
        .await
        .map(|song| (id.clone(), song, meta));
    Some(r)
}

/// The sources of a session, decoded on the page: the memory path, when
/// browser storage failed (decision 0015).  The stems are not read here:
/// the open shows them as stored lanes (D11 of the M7 plan).
async fn rebuild(base: &str, meta: &Meta) -> Result<Song> {
    let mut files = Vec::new();
    for (orig, stored) in &meta.files {
        let bytes = read(&format!("{base}/src/{stored}"))
            .await
            .ok_or_else(|| anyhow!("{orig} is missing from browser storage"))?;
        files.push((orig.clone(), bytes));
    }
    Song::load_files(files)
}

/// The suffix of each file that the decode worker writes.
const DECODE_PART: &str = ".decode.part";

/// The overviews of a session's stems, in the order of its `stem` lines,
/// and of their mix (D7 of the M7 plan).
pub struct StemOverviewSet {
    pub lanes: Vec<std::rc::Rc<crate::lanes::Overview>>,
    pub mix: std::rc::Rc<crate::lanes::Overview>,
}

/// The first bytes of the file at `path`, at most `most`, and its size.
async fn head_of(path: &str, most: u64) -> Option<(Vec<u8>, u64)> {
    let file = file_handle(path).await?;
    let size = file.size() as u64;
    let bytes: Uint8Array = st_slice(&file, 0.0, size.min(most) as f64)
        .await
        .ok()?
        .unchecked_into();
    let mut head = vec![0; bytes.length() as usize];
    bytes.copy_to(&mut head);
    Some((head, size))
}

/// The lane file of each stem of `meta` in session `id`, from its WAV's
/// header (at most 4 KiB read): `None` for a stem whose file is not
/// complete (D8 of M6).
pub async fn stem_lane_files(id: &str, meta: &Meta, frames: usize) -> Vec<Option<LaneFile>> {
    let mut files = Vec::with_capacity(meta.stems.len());
    for name in &meta.stems {
        let path = format!("{}/stems/{}", session_folder(id), stem_file(name));
        let file = match head_of(&path, 4_096).await {
            Some((head, size)) => crate::store_meta::stem_lane_file(&path, &head, size, frames),
            None => None,
        };
        files.push(file);
    }
    files
}

/// The overview files of `count` stems, then of their mix, from the
/// session folder.
fn stem_overview_files(count: usize) -> Vec<String> {
    (0..count)
        .map(crate::store_meta::stem_overview_file)
        .chain([crate::store_meta::STEM_MIX_OVERVIEW.to_string()])
        .collect()
}

/// The stem overviews of session `id`, for a song of `frames` frames,
/// made once (D7 of the M7 plan).  When `meta` names complete overviews of
/// the stems of now, the page reads them (at most 32 KiB each).  Else the
/// decode worker makes them from the stem WAVs, in blocks, and the page
/// moves them into place and writes the `stem-overviews` line, if the
/// stems did not change meanwhile.  If the worker cannot make them, the
/// page makes them through the lane reader, in blocks, and does not save
/// them.  `None` for a session with no stems, or a stem that is not
/// complete.
pub async fn stem_overviews(
    id: &str,
    frames: usize,
    decoder: &crate::decoder::DecodeClient,
) -> Option<StemOverviewSet> {
    match stem_overviews_or_why(id, frames, decoder).await {
        StemOverviews::Made(set) => Some(set),
        StemOverviews::PageFailed | StemOverviews::NotMade => None,
    }
}

/// The page's log line when it makes the stem overviews itself, before
/// the reason (D7's fallback).  `xtask e2e old-session` looks for it.
pub const PAGE_OVERVIEWS: &str = "decoder: the page makes the stem overviews";

/// How [`stem_overviews`] ended, so that `stem_lanes` can tell a failed
/// attempt of the page from no attempt (finding 8-1 of review round 8 of
/// M7).
enum StemOverviews {
    Made(StemOverviewSet),
    /// The page made them itself, and a read failed.
    PageFailed,
    /// Nothing was made: storage is closed, the session has no stems, a
    /// stem is not complete, or the stems changed meanwhile.
    NotMade,
}

/// [`stem_overviews`], with how it ended.
async fn stem_overviews_or_why(
    id: &str,
    frames: usize,
    decoder: &crate::decoder::DecodeClient,
) -> StemOverviews {
    use StemOverviews::{Made, NotMade, PageFailed};
    use stem_protocol::decode::{DecodeReply, OverviewLane, OverviewsRequest, WorkerRequest};
    if CLOSED.get() {
        return NotMade;
    }
    let Ok(meta) = meta_now(id).await else {
        return NotMade;
    };
    if meta.stems.is_empty() {
        return NotMade;
    }
    let base = session_folder(id);
    let overview_files = stem_overview_files(meta.stems.len());
    if stem_overviews_complete(&base, &meta, frames, &overview_files).await
        && let Some(set) = read_stem_overviews(&base, &overview_files, frames).await
    {
        return Made(set);
    }
    let Some(files) = stem_lane_files(id, &meta, frames)
        .await
        .into_iter()
        .collect::<Option<Vec<LaneFile>>>()
    else {
        return NotMade;
    };
    let generation = meta.stems_generation;
    let part = |file: &str| crate::store_meta::stem_overview_part(file, generation);
    let lanes = files
        .iter()
        .zip(&overview_files)
        .map(|(file, overview)| OverviewLane {
            lane: lane_ref(&base, file),
            overview: part(overview),
        })
        .collect();
    let request = WorkerRequest::Overviews(OverviewsRequest {
        session: base.clone(),
        frames: frames as u64,
        lanes,
        mix: overview_files.last().map(|mix| part(mix)),
    });
    let why = match decoder.request(&request, DecodeGuard::new(id)).await {
        crate::decoder::Ended::Reply(DecodeReply::OverviewsMade, guard) => {
            let kept = keep_stem_overviews(id, &meta.stems, generation, frames).await;
            drop(guard);
            if !kept {
                log::info!("decoder: the stems changed while their overviews were made");
                return NotMade;
            }
            if let Some(set) = read_stem_overviews(&base, &overview_files, frames).await {
                return Made(set);
            }
            "the overview files could not be read".to_string()
        }
        crate::decoder::Ended::Reply(DecodeReply::Unstored { text }, guard) => {
            drop(guard);
            text
        }
        crate::decoder::Ended::Reply(other, guard) => {
            drop(guard);
            format!("the decode worker replied {}", other.kind())
        }
        crate::decoder::Ended::Stopped(why) => why,
    };
    if CLOSED.get() {
        return NotMade;
    }
    log::warn!("{PAGE_OVERVIEWS}: {why}");
    overviews_on_the_page(&files, frames)
        .await
        .map_or(PageFailed, Made)
}

/// Whether `meta` names complete overviews of its stems for `frames`, and
/// each file has the size of D4 of the M7 plan.
async fn stem_overviews_complete(base: &str, meta: &Meta, frames: usize, files: &[String]) -> bool {
    if !meta.stem_overviews_for(frames) {
        return false;
    }
    let want = crate::store_meta::overview_bytes(frames) as f64;
    for file in files {
        if file_size(&format!("{base}/{file}")).await != Some(want) {
            return false;
        }
    }
    true
}

/// Reads the overview files `files` of the session folder `base`.
async fn read_stem_overviews(
    base: &str,
    files: &[String],
    frames: usize,
) -> Option<StemOverviewSet> {
    let mut overviews = Vec::with_capacity(files.len());
    for file in files {
        let bytes = read(&format!("{base}/{file}")).await?;
        match crate::lanes::Overview::from_file(&bytes, frames) {
            Ok(overview) => overviews.push(std::rc::Rc::new(overview)),
            Err(e) => {
                log::warn!("decoder: {base}/{file}: {e}");
                return None;
            }
        }
    }
    let mix = overviews.pop()?;
    Some(StemOverviewSet {
        lanes: overviews,
        mix,
    })
}

/// Moves the decode worker's overview files of the stems of `generation`
/// onto their names, and writes the `stem-overviews` line, under the
/// `meta` lock.  If the stems changed meanwhile (another generation, or
/// other names), it removes the part files instead, and gives false.
async fn keep_stem_overviews(id: &str, names: &[String], generation: u64, frames: usize) -> bool {
    saving(id, async {
        let _meta = lock_meta(id).await;
        let base = session_folder(id);
        let folder = format!("{base}/overviews/stems");
        let files = stem_overview_files(names.len());
        let name_of = |file: &str| file.rsplit('/').next().unwrap_or(file).to_string();
        let parts: Vec<(String, String)> = files
            .iter()
            .map(|file| {
                let part = crate::store_meta::stem_overview_part(file, generation);
                (name_of(&part), name_of(file))
            })
            .collect();
        let remove_parts = || async {
            for (part, _) in &parts {
                st_remove(&format!("{folder}/{part}")).await;
            }
        };
        let Ok(mut meta) = meta_now(id).await else {
            remove_parts().await;
            return false;
        };
        if meta.stems_generation != generation || meta.stems != names {
            remove_parts().await;
            return false;
        }
        for (part, name) in &parts {
            if !move_file(&folder, part, name).await {
                remove_parts().await;
                return false;
            }
        }
        meta.stem_overviews = Some(crate::store_meta::StemOverviews {
            frames,
            format: MIX_FORMAT.into(),
            generation,
        });
        write_bytes(&base, "meta", meta.to_text().as_bytes())
            .await
            .is_ok()
    })
    .await
}

/// The classic-drums WAV of a session, from its folder: a scratch file
/// that `meta` does not name (D8 of the M7 plan).
const CLASSIC_WAV: &str = "classic.wav";
/// The overview of the classic-drums WAV, from the session folder.
const CLASSIC_OVERVIEW: &str = "overviews/classic.f32";

/// The lane of a lane file, for a request of the decode worker: its path
/// from the session folder `base`.
fn lane_ref(base: &str, file: &LaneFile) -> stem_protocol::decode::LaneRef {
    use stem_protocol::decode::LaneEncoding;
    stem_protocol::decode::LaneRef {
        path: file
            .path
            .strip_prefix(&format!("{base}/"))
            .unwrap_or(&file.path)
            .to_string(),
        encoding: match file.format {
            engine::readahead::LaneFormat::F32 => LaneEncoding::F32,
            engine::readahead::LaneFormat::Pcm16 => LaneEncoding::Pcm16,
        },
        data_offset: file.data_offset,
        data_frames: file.data_frames,
    }
}

/// The `classic` request of session `id` for a song of `frames` frames,
/// whose input lanes are `inputs` (D8 of the M7 plan).
pub fn classic_request(
    id: &str,
    frames: usize,
    inputs: &[LaneFile],
) -> stem_protocol::decode::WorkerRequest {
    let base = session_folder(id);
    stem_protocol::decode::WorkerRequest::Classic(stem_protocol::decode::ClassicRequest {
        session: base.clone(),
        frames: frames as u64,
        lanes: inputs.iter().map(|file| lane_ref(&base, file)).collect(),
        wav: format!("{CLASSIC_WAV}{DECODE_PART}"),
        overview: format!("{CLASSIC_OVERVIEW}{DECODE_PART}"),
    })
}

/// After `classic-made`: moves the decode worker's WAV and overview of
/// classic drums onto their names in session `id`, and gives the lane
/// file of the WAV, from its header, and its overview (D8 of the M7
/// plan).
pub async fn keep_classic(id: &str, frames: usize) -> Result<(LaneFile, crate::lanes::Overview)> {
    saving(id, async {
        let base = session_folder(id);
        let overview_folder = format!("{base}/overviews");
        let overview_name = CLASSIC_OVERVIEW
            .rsplit('/')
            .next()
            .unwrap_or(CLASSIC_OVERVIEW);
        let moved = move_file(&base, &format!("{CLASSIC_WAV}{DECODE_PART}"), CLASSIC_WAV).await
            && move_file(
                &overview_folder,
                &format!("{overview_name}{DECODE_PART}"),
                overview_name,
            )
            .await;
        if !moved {
            st_remove(&format!("{base}/{CLASSIC_WAV}{DECODE_PART}")).await;
            st_remove(&format!("{base}/{CLASSIC_OVERVIEW}{DECODE_PART}")).await;
            return Err(anyhow!(
                "browser storage could not move the classic drums into place"
            ));
        }
        let path = format!("{base}/{CLASSIC_WAV}");
        let (head, size) = head_of(&path, 4_096)
            .await
            .ok_or_else(|| anyhow!("the classic drums are missing"))?;
        let file = crate::store_meta::stem_lane_file(&path, &head, size, frames)
            .ok_or_else(|| anyhow!("the classic drums are not a complete WAV"))?;
        let bytes = read(&format!("{base}/{CLASSIC_OVERVIEW}"))
            .await
            .ok_or_else(|| anyhow!("the overview of the classic drums is missing"))?;
        let overview = crate::lanes::Overview::from_file(&bytes, frames).map_err(|e| anyhow!(e))?;
        Ok((file, overview))
    })
    .await
}

/// D7 of the M7 plan, when the decode worker cannot: the page makes the
/// overviews of the stems in `files`, and of their mix at volume 1,
/// through the lane reader, a block at a time.  It does not save them.
async fn overviews_on_the_page(files: &[LaneFile], frames: usize) -> Option<StemOverviewSet> {
    use crate::lanes::{LaneAudio, LanePcm, LanesMix, Overview};
    use engine::mixdown::{MixRecipe, Pcm};
    let store: std::rc::Rc<dyn crate::lane_store::LaneStore> =
        std::rc::Rc::new(crate::lane_store::OpfsStore::new(None));
    let lanes: Vec<LaneAudio> = files
        .iter()
        .map(|file| LaneAudio::Stored {
            file: file.clone(),
            overview: std::rc::Rc::new(Overview {
                columns: Vec::new(),
                scale: 1.0,
            }),
            store: std::rc::Rc::clone(&store),
        })
        .collect();
    let overview_of = |pcm: Box<dyn Pcm>| async move {
        let mut job = crate::export::overview_job(pcm, frames);
        while !job.run_while(crate::export::time_left(4.0)) {
            st_sleep(1).await;
        }
        if let Some(e) = job.failed() {
            log::warn!("decoder: a stem could not be read for its overview: {e}");
            return None;
        }
        let waveform = job.into_sink().finish();
        Some(std::rc::Rc::new(Overview {
            columns: waveform.columns,
            scale: waveform.scale,
        }))
    };
    let mut overviews = Vec::with_capacity(lanes.len());
    for lane in &lanes {
        overviews.push(overview_of(Box::new(LanePcm::new(lane.clone(), frames, 44_100))).await?);
    }
    let all = MixRecipe::new(vec![1.0; lanes.len()]);
    let mix = overview_of(Box::new(LanesMix::new(&lanes, frames, 44_100, &all))).await?;
    Some(StemOverviewSet {
        lanes: overviews,
        mix,
    })
}

/// A decode of session `id` by the decode worker, from the post to the end
/// of D7 (D9 of `plans/active/stream-from-storage-m5.md`).  It counts as a
/// save in `SAVING`, so `forget` and a restore wait for it, and as a write
/// in `WRITES`, so [`close`] waits for it.  A reset ends the decode and
/// drops the guard first, so it never waits for a long decode.
pub struct DecodeGuard {
    _saving: SaveHeld,
    id: String,
}

impl DecodeGuard {
    pub fn new(id: &str) -> Self {
        WRITES.set(WRITES.get() + 1);
        DECODING.with_borrow_mut(|d| *d.entry(id.to_string()).or_default() += 1);
        Self {
            _saving: SaveHeld::new(id),
            id: id.to_string(),
        }
    }
}

impl Drop for DecodeGuard {
    fn drop(&mut self) {
        WRITES.set(WRITES.get() - 1);
        DECODING.with_borrow_mut(|d| {
            if let Some(n) = d.get_mut(&self.id) {
                *n -= 1;
                if *n == 0 {
                    d.remove(&self.id);
                }
            }
        });
    }
}

/// Writes in progress, which [`close`] waits for.
#[cfg(test)]
#[allow(dead_code, reason = "the reset test in lib.rs reads it")]
pub(crate) fn writes_in_progress() -> usize {
    WRITES.get()
}

/// Whether the decode worker decodes session `id` now.
pub fn decoding(id: &str) -> bool {
    DECODING.with_borrow(|d| d.contains_key(id))
}

/// Whether browser storage is closed for a reset: no decode is posted.
pub fn is_closed() -> bool {
    CLOSED.get()
}

/// Records the session of the separation that may read or write its
/// `mix.f32` (any stage but the memory path), or none.  The page sets it in
/// each frame, so D7 can see it (D8).
pub fn set_separating(id: Option<String>) {
    SEPARATING.with_borrow_mut(|s| *s = id);
    run_deferred();
}

/// Whether a separation or a mix write names session `id` (D8): D7 then
/// keeps the session's `mix.f32` as it is.
pub fn mix_in_use(id: &str) -> bool {
    SEPARATING.with_borrow(|s| s.as_deref() == Some(id))
        || MIX_WRITING.with_borrow(|w| w.contains_key(id))
}

/// Records the session that the shown song plays a lane from, or none
/// (D10 of M6).  `load_song` sets it.
pub fn set_playing(id: Option<String>) {
    PLAYING.with_borrow_mut(|p| *p = id);
    run_deferred();
}

/// Whether no failure may remove a file of session `id` now: the shown
/// song plays from it, or an open reads it (D10 of M6).
pub fn protected(id: &str) -> bool {
    PLAYING.with_borrow(|p| p.as_deref() == Some(id)) || OPENING.with_borrow(|o| o.contains_key(id))
}

/// An open of session `id`, which protects the session from when the open
/// starts until the song is shown or the open ends (D10 of M6).
pub struct OpenGuard(String);

impl OpenGuard {
    pub fn new(id: &str) -> Self {
        OPENING.with_borrow_mut(|o| *o.entry(id.to_string()).or_default() += 1);
        Self(id.to_string())
    }
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        OPENING.with_borrow_mut(|o| {
            if let Some(n) = o.get_mut(&self.0) {
                *n -= 1;
                if *n == 0 {
                    o.remove(&self.0);
                }
            }
        });
        run_deferred();
    }
}

/// Cancels a mix removal that waits for session `id`: a separation posts
/// that mix, so the worker reads it (D10 of M6).
pub fn keep_mix(id: &str) {
    DEFERRED.with_borrow_mut(|d| d.remove(id));
}

/// Whether a mix removal waits for session `id`, for a test.
#[cfg(test)]
pub(crate) fn mix_removal_waits(id: &str) -> bool {
    DEFERRED.with_borrow(|d| d.contains(id))
}

/// Runs each mix removal that waited, once its session is neither
/// protected nor named by a separation or a mix write.
fn run_deferred() {
    let ready: Vec<String> = DEFERRED.with_borrow(|d| {
        d.iter()
            .filter(|id| !protected(id) && !mix_in_use(id))
            .cloned()
            .collect()
    });
    for id in ready {
        DEFERRED.with_borrow_mut(|d| d.remove(&id));
        let removal = forget_mix(&id);
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = removal.await {
                log::warn!("store: could not remove the mix of session {id}: {e:#}");
            }
        });
    }
}

/// `meta` of session `id`, read with no wait for the saves into the
/// session: the caller holds the session's [`DecodeGuard`] (D8a).
async fn meta_now(id: &str) -> Result<Meta> {
    let bytes = read(&format!("{}/meta", session_folder(id)))
        .await
        .ok_or_else(|| anyhow!("session {id} is gone"))?;
    Ok(Meta::parse(&String::from_utf8_lossy(&bytes)))
}

/// Removes the `decoded` line of session `id`, if it has one, before a
/// decode (D2).  It holds the session as a save meanwhile, and waits for
/// no other save.
pub async fn clear_decoded(id: &str) -> Result<()> {
    saving(id, async {
        let _meta = lock_meta(id).await;
        let mut meta = meta_now(id).await?;
        if meta.decoded.take().is_some() {
            write_bytes(&session_folder(id), "meta", meta.to_text().as_bytes()).await?;
        }
        Ok(())
    })
    .await
}

/// What D7 did with the decode worker's files.
#[derive(Debug, PartialEq)]
pub enum Kept {
    /// The session is complete: its files are in place, and `meta` has the
    /// `decoded` line.
    Complete,
    /// A separation or a mix write names the session, and its `mix.f32` is
    /// not usable now (D8).  The worker's files are removed, and the
    /// session stays incomplete.
    MixBusy,
}

/// D7, after a `decoded` reply for session `id`: moves each part file onto
/// its name, then writes the `decoded` line, and the `mix` line if a new
/// `mix.f32` moved into place.  The caller holds the session's
/// [`DecodeGuard`], so this waits for no other save (D8a).  If the session
/// has a usable mix of the song's frames now ([`usable_mix_frames`]), or a
/// separation or a mix write names the session, the worker's mix is
/// removed and the session's `mix.f32` stays (D8).  Otherwise the worker's
/// mix takes its place.  An error leaves the session incomplete.
pub async fn keep_decoded(
    id: &str,
    sources: &[PlannedSource],
    frames: usize,
    new_mix: bool,
    channels: &[u8],
) -> Result<Kept> {
    // Held from the read of `meta` to its write; the moves are short.
    let _meta = lock_meta(id).await;
    let base = session_folder(id);
    let mut meta = meta_now(id).await?;
    let files: Vec<String> = crate::store_meta::session_files(frames, sources)
        .into_iter()
        .map(|(file, _)| file)
        .collect();
    let mut moved_mix = false;
    // A usable mix of the song stays; so does a mix that a separation or a
    // mix write names, usable or not (D8).
    let usable = usable_mix_frames(id, &meta).await == Some(frames);
    if new_mix && (usable || mix_in_use(id)) {
        st_remove(&format!("{base}/{MIX_FILE}{DECODE_PART}")).await;
        if !usable {
            for file in &files {
                st_remove(&format!("{base}/{file}{DECODE_PART}")).await;
            }
            return Ok(Kept::MixBusy);
        }
    } else if new_mix {
        move_part(&base, MIX_FILE).await?;
        moved_mix = true;
    }
    for file in files.iter().filter(|file| file.as_str() != MIX_FILE) {
        move_part(&base, file).await?;
    }
    meta.decoded = Some(Decoded {
        frames,
        format: MIX_FORMAT.into(),
        channels: sources
            .iter()
            .map(|s| s.index)
            .zip(channels.iter().copied())
            .collect(),
    });
    if moved_mix {
        meta.mix = Some((frames, MIX_FORMAT.into()));
    }
    write_bytes(&base, "meta", meta.to_text().as_bytes()).await?;
    Ok(Kept::Complete)
}

/// Moves `<file>.decode.part` onto `<file>` in the folder `base`.
async fn move_part(base: &str, file: &str) -> Result<()> {
    let (folder, name) = match file.rsplit_once('/') {
        Some((sub, name)) => (format!("{base}/{sub}"), name),
        None => (base.to_string(), file),
    };
    if move_file(&folder, &format!("{name}{DECODE_PART}"), name).await {
        Ok(())
    } else {
        Err(anyhow!(
            "browser storage could not move {base}/{file}{DECODE_PART} into place"
        ))
    }
}

/// D11 of the M7 plan: the song of complete session `id` as stored lanes:
/// each source's session file with its overview (`overviews/<i>.f32`), and
/// the mix lane's overview.  It reads only the overviews, at most 32 KiB
/// each, and no session file: the jobs and playback read them in blocks.
pub async fn stored_sources(
    id: &str,
    sources: &[PlannedSource],
    decoded: &Decoded,
) -> Result<crate::lanes::Shown> {
    use crate::lanes::{LaneAudio, ShownLane};
    let base = session_folder(id);
    let frames = decoded.frames;
    let files = crate::store_meta::source_lane_files(&base, sources, frames);
    let store: std::rc::Rc<dyn crate::lane_store::LaneStore> =
        std::rc::Rc::new(crate::lane_store::OpfsStore::new(None));
    let mut lanes = Vec::with_capacity(sources.len());
    for (source, file) in sources.iter().zip(files) {
        let overview =
            read_overview(&format!("{base}/overviews/{}.f32", source.index), frames).await?;
        lanes.push(ShownLane {
            name: source.name.clone(),
            audio: LaneAudio::Stored {
                file,
                overview,
                store: std::rc::Rc::clone(&store),
            },
        });
    }
    let mix = read_overview(&format!("{base}/overviews/mix.f32"), frames).await?;
    Ok(crate::lanes::Shown {
        sample_rate: engine::resample::SONG_RATE,
        frames,
        lanes,
        mix_overview: Some(mix),
    })
}

/// The overview file at `path`, of a lane of `frames` frames.
async fn read_overview(path: &str, frames: usize) -> Result<std::rc::Rc<crate::lanes::Overview>> {
    let bytes = read(path)
        .await
        .ok_or_else(|| anyhow!("{path} is missing from browser storage"))?;
    let overview =
        crate::lanes::Overview::from_file(&bytes, frames).map_err(|e| anyhow!("{path}: {e}"))?;
    Ok(std::rc::Rc::new(overview))
}

/// The stems `names` of session `id`, at `paths`, for a song of `frames`
/// frames, as lanes (D11 of the M7 plan), and the overview of their mix
/// when every stem is complete.  A complete stem WAV is a stored lane: its
/// header (at most 4 KiB read) and its overview (D7), or an empty overview
/// when none can be made.  A stem whose file is not complete (`.part`, or
/// a wrong header) is read whole into memory only on the memory path's
/// rules: a song of at most `limit` frames; else it is an error.
pub async fn stem_lanes(
    id: &str,
    names: &[String],
    paths: &[String],
    frames: usize,
    decoder: &crate::decoder::DecodeClient,
    limit: usize,
) -> Result<(
    Vec<crate::lanes::ShownLane>,
    Option<std::rc::Rc<crate::lanes::Overview>>,
)> {
    use crate::lanes::{LaneAudio, ShownLane};
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        files.push(match head_of(path, 4_096).await {
            Some((head, size)) => crate::store_meta::stem_lane_file(path, &head, size, frames),
            None => None,
        });
    }
    let complete = files.iter().all(Option::is_some);
    let set = if complete {
        stem_overviews_or_why(id, frames, decoder).await
    } else {
        StemOverviews::NotMade
    };
    let (overviews, mix) = match set {
        StemOverviews::Made(set) if set.lanes.len() == names.len() => (
            set.lanes.into_iter().map(Some).collect::<Vec<_>>(),
            Some(set.mix),
        ),
        set => {
            // The overviews of the complete stems, made on the page; their
            // mix's only when every stem is complete.  Not again after a
            // failed attempt of the page, nor while storage closes (finding
            // 8-1 of review round 8 of M7).
            let stored: Vec<LaneFile> = files.iter().flatten().cloned().collect();
            let tried = matches!(set, StemOverviews::PageFailed);
            let made = if stored.is_empty() || tried || CLOSED.get() {
                None
            } else {
                overviews_on_the_page(&stored, frames).await
            };
            let mix = made
                .as_ref()
                .filter(|_| complete)
                .map(|set| std::rc::Rc::clone(&set.mix));
            let mut lanes = made.map(|set| set.lanes.into_iter());
            let overviews = files
                .iter()
                .map(|file| file.as_ref().and_then(|_| lanes.as_mut()?.next()))
                .collect();
            (overviews, mix)
        }
    };
    let store: std::rc::Rc<dyn crate::lane_store::LaneStore> =
        std::rc::Rc::new(crate::lane_store::OpfsStore::new(None));
    let mut lanes = Vec::with_capacity(names.len());
    for (((name, path), file), overview) in names.iter().zip(paths).zip(files).zip(overviews) {
        let audio = match (file, overview) {
            // With no overview, the lane's waveform is empty: nothing draws
            // it, and the mix lane's follower reads its blocks (finding 1-2
            // of review round 1 of M7).
            (Some(file), overview) => LaneAudio::Stored {
                file,
                overview: overview.unwrap_or_else(|| {
                    std::rc::Rc::new(crate::lanes::Overview {
                        columns: Vec::new(),
                        scale: 1.0,
                    })
                }),
                store: std::rc::Rc::clone(&store),
            },
            (None, _) if frames <= limit => {
                let (stem, _) = read_stem(path, name, frames).await?;
                LaneAudio::Memory {
                    left: stem.left,
                    right: stem.right,
                }
            }
            (None, _) => {
                return Err(anyhow!(
                    "the stem {name} is not complete in browser storage, and the song is too long to read it into memory"
                ));
            }
        };
        lanes.push(ShownLane {
            name: name.clone(),
            audio,
        });
    }
    Ok((lanes, mix))
}

/// The paths of the stem WAVs of `names` in session `id`.
pub fn stem_paths(id: &str, names: &[String]) -> Vec<String> {
    names
        .iter()
        .map(|name| format!("{}/stems/{}", session_folder(id), stem_file(name)))
        .collect()
}

/// A stored session, as the page opens it (D11).
pub enum StoredSong {
    /// Its session files are complete: the page shows them as stored lanes
    /// (D11 of the M7 plan).
    Complete {
        meta: Meta,
        sources: Vec<PlannedSource>,
        decoded: Decoded,
    },
    /// The decode worker decodes it first, keeping a usable mix of
    /// `mix_frames` (D8).
    NeedsDecode {
        meta: Meta,
        sources: Vec<PlannedSource>,
        mix_frames: Option<usize>,
    },
}

/// Session `id`, as the page opens it: complete or not (D2).  `None` when
/// it has no `meta`.  It waits for the saves into the session first.
pub async fn open_stored(id: &str) -> Option<Result<StoredSong>> {
    settled(id).await;
    let base = session_folder(id);
    let meta = Meta::parse(&String::from_utf8_lossy(
        &read(&format!("{base}/meta")).await?,
    ));
    let sources = match crate::store_meta::planned_sources(&meta) {
        Ok(sources) if sources.is_empty() => return Some(Err(anyhow!("no stems to load"))),
        Ok(sources) => sources,
        Err(e) => return Some(Err(anyhow!(e))),
    };
    let mut sizes = HashMap::new();
    if let Some(decoded) = &meta.decoded {
        for (file, _) in crate::store_meta::session_files(decoded.frames, &sources) {
            if let Some(size) = file_size(&format!("{base}/{file}")).await {
                sizes.insert(file, size as u64);
            }
        }
    }
    let complete =
        crate::store_meta::complete_frames(&meta, &sources, |file| sizes.get(file).copied());
    Some(Ok(match (complete, meta.decoded.clone()) {
        (Some(_), Some(decoded)) => StoredSong::Complete {
            meta,
            sources,
            decoded,
        },
        _ => {
            let mix_frames = usable_mix_frames(id, &meta).await;
            StoredSong::NeedsDecode {
                meta,
                sources,
                mix_frames,
            }
        }
    }))
}

/// The frames of the usable mix file of session `id`, whose `meta` is
/// `meta` (D1 of M4): a `mix` line in [`MIX_FORMAT`], and a file of 8 bytes
/// for each frame.  It waits for no save.
pub async fn usable_mix_frames(id: &str, meta: &Meta) -> Option<usize> {
    let frames = meta.usable_mix()?;
    (file_size(&mix_path(id)).await? == 8.0 * frames as f64).then_some(frames)
}

/// Makes session `id` the one that a reload restores.
pub async fn make_current(id: &str) {
    let written = write(&format!("{ROOT}/current"), id.as_bytes()).await;
    if written.is_err() {
        log::warn!("could not mark session {id} as current");
    }
    #[cfg(test)]
    if written.is_ok() {
        MADE_CURRENT.with_borrow_mut(|made| *made = Some(id.to_string()));
    }
}

#[cfg(test)]
thread_local! {
    /// The session that [`make_current`] wrote last, for a test.
    static MADE_CURRENT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The session that [`make_current`] wrote last, which a test reads at a
/// log line, where it cannot wait for browser storage.
#[cfg(test)]
pub(crate) fn test_made_current() -> Option<String> {
    MADE_CURRENT.with_borrow(Clone::clone)
}

/// The lane file of the usable mix of session `id`, if it has the song's
/// `frames`: the "original" lane of a song of several sources (D8 of M6).
pub async fn mix_lane(id: &str, meta: &Meta, frames: usize) -> Option<LaneFile> {
    (usable_mix_frames(id, meta).await? == frames)
        .then(|| crate::store_meta::mix_lane_file(&session_folder(id), frames))
}

/// [`mix_lane`], with `meta` read now.
pub async fn mix_lane_now(id: &str, frames: usize) -> Option<LaneFile> {
    let meta = meta_now(id).await.ok()?;
    mix_lane(id, &meta, frames).await
}

/// The file at `path` as a `File`, which holds no bytes until it is read:
/// the reader of a stored lane gets it once, and again after a read fails.
pub async fn file_handle(path: &str) -> Option<web_sys::File> {
    let file = st_file(path).await;
    (!file.is_null()).then(|| file.unchecked_into())
}

/// Bytes used and available to this site, as the browser estimates them.
pub async fn usage() -> (f64, f64) {
    let v: Array = st_usage().await.unchecked_into();
    (
        v.get(0).as_f64().unwrap_or(0.0),
        v.get(1).as_f64().unwrap_or(0.0),
    )
}

/// Offers `bytes` to the user as a download named `name`.
pub fn download(name: &str, bytes: &Uint8Array, mime: &str) {
    save_file(name, bytes, mime);
}

/// Offers `blob` as a download named `name`, with no copy of its bytes.
pub fn download_blob(name: &str, blob: &web_sys::Blob) {
    save_blob(name, blob);
}

/// Writes `bytes` to the file at `path` from the root, for a test.
#[cfg(test)]
pub(crate) async fn test_write(path: &str, bytes: &[u8]) -> Result<()> {
    write(path, bytes).await
}

/// Opens storage again after a test closed it with [`close`].
#[cfg(test)]
pub(crate) fn test_reopen() {
    CLOSED.set(false);
}

/// Removes the file at `path` from the root, for a test.
#[cfg(test)]
pub(crate) async fn test_remove(path: &str) -> bool {
    st_remove(path).await.as_bool() == Some(true)
}

/// Waits until no save into session `id` runs, a mix removal included, for
/// a test.
#[cfg(test)]
pub(crate) async fn test_settled(id: &str) {
    settled(id).await;
}

/// Removes every stored session, for a test.
#[cfg(test)]
pub(crate) async fn test_wipe() {
    st_remove(ROOT).await;
}

/// The bytes of the file at `path` from the root, for a test.
#[cfg(test)]
pub(crate) async fn test_read(path: &str) -> Option<Vec<u8>> {
    read(path).await
}

/// Browser tests of the private write path (`cargo run -p xtask --
/// web-test`).  `tests/web.rs` and `tests/web_close.rs` test the public
/// functions, except `usage`, `download` and `download_blob` (TD-53).
#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    thread_local! {
        /// Heap bytes allocated and not yet freed.
        static LIVE: Cell<usize> = const { Cell::new(0) };
        /// The most `LIVE` has been since a test set it.
        static PEAK: Cell<usize> = const { Cell::new(0) };
    }

    /// Keeps `LIVE` and `PEAK` for every test in this binary.  Only `alloc`
    /// and `dealloc` are written, so the default `realloc` holds the old
    /// and the new buffer at once, as `System`'s may.
    struct PeakAllocator;

    // SAFETY: every call is forwarded unchanged to `System`; counting
    // touches only `const` thread-locals, which do not allocate.
    unsafe impl std::alloc::GlobalAlloc for PeakAllocator {
        unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
            let _ = LIVE.try_with(|live| {
                let now = live.get() + layout.size();
                live.set(now);
                let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
            });
            // SAFETY: the caller's contract is the same as `System`'s.
            unsafe { std::alloc::System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
            let _ = LIVE.try_with(|live| live.set(live.get().saturating_sub(layout.size())));
            // SAFETY: the caller's contract is the same as `System`'s.
            unsafe { std::alloc::System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: PeakAllocator = PeakAllocator;

    #[wasm_bindgen(inline_js = r#"
let record = null;
let real = null;
// Records what each OPFS write gets, in call order: "blob", "whole" (a
// typed array that is its whole buffer) or "view".
// "same" is a write of `expected` itself.  From call `fail_from` on (0 is
// the first), a write is refused, as a full disk refuses it.
export function start_spy(expected, fail_from) {
  if (real) throw new Error("a spy is already on write");
  const proto = FileSystemWritableFileStream.prototype;
  real = proto.write;
  record = [];
  proto.write = function (data) {
    if (record.length >= fail_from) {
      record.push("refused");
      return Promise.reject(new DOMException("refused by the test", "QuotaExceededError"));
    }
    if (data === expected) record.push("same");
    else if (data instanceof Blob) record.push("blob");
    else if (ArrayBuffer.isView(data))
      record.push(data.byteOffset === 0 && data.byteLength === data.buffer.byteLength ? "whole" : "view");
    else record.push(typeof data);
    return real.call(this, data);
  };
}
// Restores `write` and returns the record, joined by spaces.
export function stop_spy() {
  FileSystemWritableFileStream.prototype.write = real;
  real = null;
  return record.join(" ");
}
// The next `createWritable` is refused, after the file is made.
export function refuse_next_writable() {
  const proto = FileSystemFileHandle.prototype;
  const real = proto.createWritable;
  proto.createWritable = function () {
    proto.createWritable = real;
    return Promise.reject(new DOMException("refused by the test", "NoModificationAllowedError"));
  };
}
// "lloyd" at `at` in a buffer of 16 bytes, as a view of 5 bytes.
export function lloyd_in_a_larger_buffer(at) {
  const b = new Uint8Array(16);
  b.set(new TextEncoder().encode("lloyd"), at);
  return b.subarray(at, at + 5);
}
"#)]
    extern "C" {
        fn start_spy(expected: &JsValue, fail_from: f64);
        fn stop_spy() -> String;
        fn lloyd_in_a_larger_buffer(at: u32) -> Uint8Array;
        fn refuse_next_writable();
    }

    /// Records every write, refusing none.
    fn spy() {
        start_spy(&JsValue::UNDEFINED, f64::INFINITY);
    }

    /// MEM-5: a view reaches storage as a whole array.  The spy sees what
    /// `write` gets, so a view fails this in any browser; a read-back
    /// alone fails only in Safari (`docs/references/browser-quirks.md`).
    /// The views start at the buffer's start, in its middle, and end at its
    /// end, so `whole` must check both the offset and the length.
    #[wasm_bindgen_test]
    async fn a_view_is_written_whole() {
        st_remove(ROOT).await;
        let starts = [0, 4, 11];
        let mut written = Vec::new();
        spy();
        for at in starts {
            let path = format!("sessions/v/{at}");
            written.push(st_write("", &path, &lloyd_in_a_larger_buffer(at)).await);
        }
        let record = stop_spy();
        assert!(written.iter().all(|w| w.as_bool() == Some(true)));
        assert_eq!(record, "whole whole whole");
        for at in starts {
            let back = read(&format!("sessions/v/{at}")).await;
            assert_eq!(back.as_deref(), Some(&b"lloyd"[..]), "view at {at}");
        }
        st_remove(ROOT).await;
    }

    fn blob(bytes: &[u8]) -> web_sys::Blob {
        let parts = Array::of1(&Uint8Array::from(bytes));
        web_sys::Blob::new_with_u8_array_sequence(&parts).expect("a blob")
    }

    /// A save refused after the song's file (here, at `meta`) removes the
    /// session, which would otherwise hold the file and never be listed.
    #[wasm_bindgen_test]
    async fn a_failed_save_leaves_no_session() {
        st_remove(ROOT).await;
        start_spy(&JsValue::UNDEFINED, 1.0);
        let saved = save_sources(&[("acme.wav".into(), blob(&tone(80)))], 44_100).await;
        let record = stop_spy();
        assert_eq!(record, "blob refused");
        let error = saved.expect_err("refused").to_string();
        assert!(error.ends_with("/meta (QuotaExceededError)"), "{error}");
        assert_eq!(st_list(ROOT).await.unchecked_into::<Array>().length(), 0);
        st_remove(ROOT).await;
    }

    #[wasm_bindgen(inline_js = r#"
let real_now = null;
export function freeze_now(ms) { real_now = Date.now; Date.now = () => ms; }
export function thaw_now() { Date.now = real_now; real_now = null; }
"#)]
    extern "C" {
        fn freeze_now(ms: f64);
        fn thaw_now();
    }

    /// Two saves in the same millisecond get two sessions, so a failure in
    /// one could never remove the other's folder.
    #[wasm_bindgen_test]
    async fn two_saves_in_one_millisecond_get_two_sessions() {
        st_remove(ROOT).await;
        freeze_now(1_792_640_000_000.0);
        let first = save_sources(&[("acme.wav".into(), blob(&tone(80)))], 44_100).await;
        let second = save_sources(&[("beachape.wav".into(), blob(&tone(80)))], 44_100).await;
        thaw_now();
        let (first, second) = (first.expect("saved"), second.expect("saved"));
        assert_ne!(first, second);
        assert_eq!(list().await.len(), 2);
        st_remove(ROOT).await;
    }

    /// After a reload the last id is gone, so a clock that went back can
    /// give a stored session's id.  The save takes the next one, and a
    /// failure then cannot remove the stored session.
    #[wasm_bindgen_test]
    async fn a_save_never_takes_a_stored_session_id() {
        st_remove(ROOT).await;
        let stored = "1792640000000";
        write(&format!("{ROOT}/{stored}/meta"), b"name\tacme\n")
            .await
            .expect("written");
        LAST_ID.set(0);
        freeze_now(1_792_640_000_000.0);
        start_spy(&JsValue::UNDEFINED, 0.0);
        let saved = save_sources(&[("lloyd.wav".into(), blob(&tone(80)))], 44_100).await;
        let record = stop_spy();
        thaw_now();
        assert_eq!(record, "refused");
        assert!(saved.is_err());
        assert!(read(&format!("{ROOT}/{stored}/meta")).await.is_some());
        st_remove(ROOT).await;
    }

    /// A stereo tone as a WAV file of `frames` frames at 44.1 kHz.
    fn tone(frames: usize) -> Vec<u8> {
        let wave: Vec<f32> = (0..frames).map(|i| 0.25 * (0.3 * i as f32).sin()).collect();
        engine::wav::encode_pcm16(&wave, &wave, 44_100)
    }

    /// MEM-1, MEM-5: a chosen file is saved from the browser's file, so the
    /// page needs no copy of its bytes to save it.
    #[wasm_bindgen_test]
    async fn sources_are_saved_from_their_files() {
        st_remove(ROOT).await;
        let file = blob(&tone(800));
        start_spy(&file, f64::INFINITY);
        let saved = save_sources(&[("Lloyd - Tune.wav".into(), file)], 44_100).await;
        let record = stop_spy();
        let id = saved.expect("saved");
        assert_eq!(record, "same whole", "the file itself, then meta");
        // D9: the page makes the session current once its song opens.
        assert_eq!(current().await, None, "the save makes no session current");
        let (got, song, meta) = restore_id(&id).await.expect("stored").expect("readable");
        assert_eq!((got, song.frames, meta.rate), (id, 800, 44_100));
        st_remove(ROOT).await;
    }

    /// A session `id` with one source file and the `stem` lines of an
    /// earlier separation.
    async fn plant_session(id: &str, stems: &[&str]) -> Meta {
        let meta = Meta {
            name: "acme".into(),
            rate: 44_100,
            files: vec![("acme.wav".into(), "0.wav".into())],
            stems: stems.iter().map(|s| s.to_string()).collect(),
            mix: None,
            decoded: None,
            ..Meta::default()
        };
        write(&format!("{ROOT}/{id}/meta"), meta.to_text().as_bytes())
            .await
            .expect("written");
        meta
    }

    /// D5 of M3: the worker's `.part` files replace the kept stems, and
    /// `meta` names the new stems, once every file has moved.
    #[wasm_bindgen_test]
    async fn stored_stems_replace_the_kept_stems() {
        st_remove(ROOT).await;
        let id = "1792640000000";
        let before = plant_session(id, &["old"]).await;
        let stems = format!("{ROOT}/{id}/stems");
        write(&format!("{stems}/bass.wav"), &[1])
            .await
            .expect("written");
        write(&format!("{stems}/bass.wav.part"), &[2, 2])
            .await
            .expect("written");
        write(&format!("{stems}/drums.wav.part"), &[3, 3, 3])
            .await
            .expect("written");
        let names = ["bass".to_string(), "drums".to_string()];
        spy();
        let stored = keep_stored_stems(id, &names).await;
        let record = stop_spy();
        assert_eq!(record, "whole whole", "MEM-5: the two meta writes");
        stored.kept.expect("kept");
        assert_eq!(
            stored.paths,
            [format!("{stems}/bass.wav"), format!("{stems}/drums.wav")]
        );
        assert_eq!(read(&format!("{stems}/bass.wav")).await, Some(vec![2, 2]));
        assert_eq!(
            read(&format!("{stems}/drums.wav")).await,
            Some(vec![3, 3, 3])
        );
        for part in ["bass.wav.part", "drums.wav.part"] {
            assert_eq!(read(&format!("{stems}/{part}")).await, None, "{part}");
        }
        let meta = Meta::parse(&String::from_utf8_lossy(
            &read(&format!("{ROOT}/{id}/meta")).await.expect("meta"),
        ));
        assert_eq!(meta.stems, names);
        assert_eq!((meta.files, meta.name), (before.files, before.name));
        st_remove(ROOT).await;
    }

    /// D5 of M3: when a file cannot move, `meta` names no stems, so a
    /// restore never mixes the stems of two separations.  The page reads
    /// that stem from its `.part` file.
    #[wasm_bindgen_test]
    async fn a_stem_that_cannot_move_leaves_no_stem_lines() {
        st_remove(ROOT).await;
        let id = "1792640000001";
        plant_session(id, &["bass", "drums"]).await;
        let stems = format!("{ROOT}/{id}/stems");
        write(&format!("{stems}/bass.wav.part"), &[2, 2])
            .await
            .expect("written");
        let names = ["bass".to_string(), "drums".to_string()];
        let stored = keep_stored_stems(id, &names).await;
        let error = stored.kept.expect_err("not kept").to_string();
        assert!(error.contains("drums.wav.part"), "{error}");
        assert_eq!(
            stored.paths,
            [
                format!("{stems}/bass.wav"),
                format!("{stems}/drums.wav.part")
            ]
        );
        let meta = Meta::parse(&String::from_utf8_lossy(
            &read(&format!("{ROOT}/{id}/meta")).await.expect("meta"),
        ));
        assert!(meta.stems.is_empty(), "{:?}", meta.stems);
        assert_eq!(meta.files.len(), 1, "the song stays");
        st_remove(ROOT).await;
    }

    /// Finding 2-5 of review round 2: when the moves end and `meta` cannot
    /// name the new stems, `meta` names none, so a restore never mixes two
    /// separations.  The second write of `meta` is refused.
    #[wasm_bindgen_test]
    async fn a_stop_after_the_moves_leaves_no_stem_lines() {
        st_remove(ROOT).await;
        let id = "1792640000002";
        plant_session(id, &["bass"]).await;
        let stems = format!("{ROOT}/{id}/stems");
        write(&format!("{stems}/bass.wav"), &[1])
            .await
            .expect("written");
        write(&format!("{stems}/bass.wav.part"), &[2, 2])
            .await
            .expect("written");
        start_spy(&JsValue::UNDEFINED, 1.0);
        let stored = keep_stored_stems(id, &["bass".to_string()]).await;
        stop_spy();
        assert!(stored.kept.is_err(), "not kept");
        assert_eq!(
            read(&format!("{stems}/bass.wav")).await,
            Some(vec![2, 2]),
            "moved"
        );
        let meta = Meta::parse(&String::from_utf8_lossy(
            &read(&format!("{ROOT}/{id}/meta")).await.expect("meta"),
        ));
        assert!(meta.stems.is_empty(), "{:?}", meta.stems);
        st_remove(ROOT).await;
    }

    /// Finding 2-5: when `meta` cannot drop the old `stem` lines, nothing
    /// moves, so the kept stems stay as they were.
    #[wasm_bindgen_test]
    async fn a_refused_meta_moves_nothing() {
        st_remove(ROOT).await;
        let id = "1792640000003";
        plant_session(id, &["bass"]).await;
        let stems = format!("{ROOT}/{id}/stems");
        write(&format!("{stems}/bass.wav"), &[1])
            .await
            .expect("written");
        write(&format!("{stems}/bass.wav.part"), &[2, 2])
            .await
            .expect("written");
        start_spy(&JsValue::UNDEFINED, 0.0);
        let stored = keep_stored_stems(id, &["bass".to_string()]).await;
        let record = stop_spy();
        assert_eq!(record, "refused", "one write of meta, refused");
        let error = stored.kept.expect_err("not kept").to_string();
        assert!(error.contains("QuotaExceededError"), "{error}");
        assert_eq!(stored.paths, [format!("{stems}/bass.wav.part")]);
        assert_eq!(read(&format!("{stems}/bass.wav")).await, Some(vec![1]));
        assert_eq!(
            read(&format!("{stems}/bass.wav.part")).await,
            Some(vec![2, 2])
        );
        let meta = Meta::parse(&String::from_utf8_lossy(
            &read(&format!("{ROOT}/{id}/meta")).await.expect("meta"),
        ));
        assert_eq!(meta.stems, ["bass"], "the kept stems stay");
        st_remove(ROOT).await;
    }

    /// D6 of M3: a stored stem reads back as its 16-bit samples, padded to
    /// the song's frames.
    #[wasm_bindgen_test]
    async fn a_stored_stem_reads_back_to_the_song_frames() {
        st_remove(ROOT).await;
        let path = format!("{ROOT}/1/stems/bass.wav");
        let left = [0.5, -0.25, 1.0 / 3.0];
        let right = [1.5, -1.0, 0.0];
        write(&path, &engine::wav::encode_pcm16(&left, &right, 44_100))
            .await
            .expect("written");
        let (stem, file) = read_stem(&path, "bass", 5).await.expect("read");
        assert!(file.is_none(), "a stem of 3 frames is no lane file of 5");
        let step = 1.0 / 32_768.0;
        assert_eq!(stem.name, "bass");
        assert_eq!(*stem.left, [0.5, -0.25, 10_923.0 * step, 0.0, 0.0]);
        assert_eq!(*stem.right, [32_767.0 * step, -1.0, 0.0, 0.0, 0.0]);
        let missing = read_stem(&format!("{ROOT}/1/stems/drums.wav"), "drums", 5).await;
        let error = missing.expect_err("no file").to_string();
        assert!(error.contains("stem drums is missing"), "{error}");
        st_remove(ROOT).await;
    }

    /// A song of one source lane, `frames` long.
    fn one_lane(frames: usize) -> Song {
        let wave = |f: f32| -> Samples {
            (0..frames)
                .map(|i| 0.7 * (f * i as f32).sin())
                .collect::<Vec<f32>>()
                .into()
        };
        Song {
            sample_rate: 44_100,
            frames,
            stems: vec![Stem {
                name: "acme".into(),
                left: wave(0.01),
                right: wave(0.023),
            }],
        }
    }

    /// A song of three source lanes, `frames` long, whose mix clips, and a
    /// derived lane that the mix leaves out.
    fn three_sources(frames: usize) -> Song {
        let lane = |name: &str, k: usize| {
            let wave = |c: usize| -> Samples {
                (0..frames)
                    .map(|i| 0.5 * ((0.004 * (k + 2 * c + 1) as f32) * i as f32).sin())
                    .collect::<Vec<f32>>()
                    .into()
            };
            Stem {
                name: name.into(),
                left: wave(0),
                right: wave(1),
            }
        };
        Song {
            sample_rate: 44_100,
            frames,
            stems: vec![
                lane("bass", 0),
                lane("drums", 1),
                lane("classic drums", 7),
                lane("vocals", 2),
            ],
        }
    }

    /// Interleaved 32-bit little-endian floats, as the mix file holds them.
    fn interleaved(left: &[f32], right: &[f32]) -> Vec<u8> {
        left.iter()
            .zip(right)
            .flat_map(|(l, r)| [l.to_le_bytes(), r.to_le_bytes()])
            .flatten()
            .collect()
    }

    /// The stems of a planted separated session.
    const STEM_NAMES: [&str; 6] = ["bass", "drums", "other", "vocals", "guitar", "piano"];

    /// A complete session `id` of a separated song of `seconds`: one
    /// source in `mix.f32`, six stem WAVs, and every overview, made in JS or
    /// small, with `meta` naming them complete.  Its frames.
    async fn plant_separated(id: &str, seconds: usize) -> usize {
        let frames = seconds * 44_100;
        let base = format!("{ROOT}/{id}");
        store_plant_tone(&format!("{base}/mix.f32"), frames as u32, false).await;
        let overview = vec![0u8; crate::store_meta::overview_bytes(frames) as usize];
        for file in ["overviews/0.f32", "overviews/mix.f32"] {
            write(&format!("{base}/{file}"), &overview)
                .await
                .expect("planted");
        }
        for (k, name) in STEM_NAMES.iter().enumerate() {
            store_plant_wav(&format!("{base}/stems/{}", stem_file(name)), frames as u32).await;
            let file = crate::store_meta::stem_overview_file(k);
            write(&format!("{base}/{file}"), &overview)
                .await
                .expect("planted");
        }
        let mix = crate::store_meta::STEM_MIX_OVERVIEW;
        write(&format!("{base}/{mix}"), &overview)
            .await
            .expect("planted");
        let meta = Meta {
            name: "acme".into(),
            rate: 44_100,
            files: vec![("acme.wav".into(), "0.wav".into())],
            stems: STEM_NAMES.iter().map(|n| n.to_string()).collect(),
            mix: Some((frames, MIX_FORMAT.into())),
            decoded: Some(Decoded {
                frames,
                format: MIX_FORMAT.into(),
                channels: vec![(0, 2)],
            }),
            stems_generation: 1,
            stem_overviews: Some(crate::store_meta::StemOverviews {
                frames,
                format: MIX_FORMAT.into(),
                generation: 1,
            }),
        };
        write(&format!("{base}/meta"), meta.to_text().as_bytes())
            .await
            .expect("planted");
        frames
    }

    /// D11 of the M7 plan, MEM-1, MEM-4: a complete session of 1 minute,
    /// then of 4 minutes, opens as stored lanes with no samples.  The
    /// read-ahead buffer is made at once (24 MiB for seven lanes), and the
    /// browser tests share one WebAssembly memory, so each open's heap peak
    /// above its start is measured: the 4-minute open's is at most 4 MB
    /// above the 1-minute open's.  No read is larger than a block of a
    /// lane.  It replaces `the_read_back_makes_no_whole_file_buffer`.
    #[wasm_bindgen_test]
    async fn a_stored_song_is_shown_with_no_samples() {
        const MB: f64 = 1e6;
        st_remove(ROOT).await;
        let client = crate::decoder::DecodeClient::new("./no-such-decode-worker.js");
        let mut opens = Vec::new();
        store_start_read_spy();
        for (id, seconds) in [("1792640000040", 60), ("1792640000041", 240)] {
            let frames = plant_separated(id, seconds).await;
            let before = LIVE.with(Cell::get);
            PEAK.with(|peak| peak.set(before));
            let (opened, _, note) = crate::restore_stored(id, &client)
                .await
                .expect("a session")
                .expect("opened");
            let mut app = crate::StemApp {
                want_device: false,
                ..crate::StemApp::default()
            };
            app.session = Some(id.to_string());
            app.open_song(opened.song);
            let peak = (PEAK.with(Cell::get) - before) as f64 / MB;
            let song = app.song.clone().expect("a song");
            let no_samples = song.lanes.iter().all(|l| l.audio.samples().is_none());
            let stored = app.playback.as_ref().map(|p| (p.stored, p.lanes));
            app.load_demo();
            opens.push((
                seconds,
                frames,
                peak,
                no_samples,
                stored,
                note,
                song.names().len(),
            ));
        }
        let reads = store_stop_read_spy();
        st_remove(ROOT).await;
        for (seconds, _, peak, no_samples, stored, note, lanes) in &opens {
            println!("open of {seconds} s: the heap peaked {peak:.1} MB above its start");
            assert!(no_samples, "{seconds} s: no lane has samples");
            assert_eq!(
                *stored,
                Some((7, 7)),
                "{seconds} s: every lane plays from storage"
            );
            assert_eq!(*note, None, "{seconds} s: not decoded on the page");
            assert_eq!(*lanes, 7, "{seconds} s: six stems and the original");
        }
        let (short, long) = (opens[0].2, opens[1].2);
        assert!(
            long <= short + 4.0,
            "MEM-1: the 4-minute open peaked {long:.1} MB, the 1-minute one {short:.1} MB"
        );
        let block = (8 * engine::source::BLOCK_FRAMES) as f64;
        assert!(reads.len() > 10, "{} reads", reads.len());
        assert!(
            reads.iter().all(|bytes| *bytes <= block),
            "no read larger than a block of a lane: {:?}",
            reads.iter().copied().fold(0.0, f64::max)
        );
    }

    /// D10 of the M7 plan: `write_mix` reads stored lanes through the lane
    /// reader, a block of a lane at a time, writes each block of the mix
    /// as a whole array of 1 MiB (MEM-5), and its heap peak stays under
    /// 4 MB above its start, for a song of 30 s (each lane is 10.6 MB).
    /// The heap counts what the page holds, which the growth of the
    /// WebAssembly memory would hide (the tests share one memory).
    #[wasm_bindgen_test]
    async fn write_mix_reads_lane_audio_in_blocks() {
        st_remove(ROOT).await;
        let id = "1792640000030";
        plant_session(id, &[]).await;
        let frames = 30 * 44_100;
        let side = |k: usize| -> Vec<f32> {
            (0..frames)
                .map(|i| 0.4 * ((0.001 + 0.002 * k as f32) * i as f32).sin())
                .collect()
        };
        let store: std::rc::Rc<dyn crate::lane_store::LaneStore> =
            std::rc::Rc::new(crate::lane_store::OpfsStore::new(None));
        let mut lanes = Vec::new();
        let mut sides = Vec::new();
        for (k, name) in ["bass", "drums", "vocals"].iter().enumerate() {
            let (left, right) = (side(k), side(k + 3));
            let path = format!("{ROOT}/{id}/decoded/{k}.f32");
            write(&path, &interleaved(&left, &right))
                .await
                .expect("planted");
            lanes.push(crate::lanes::ShownLane {
                name: name.to_string(),
                audio: crate::lanes::LaneAudio::Stored {
                    file: crate::store_meta::LaneFile::f32(path, frames),
                    overview: std::rc::Rc::new(crate::lanes::Overview {
                        columns: Vec::new(),
                        scale: 1.0,
                    }),
                    store: std::rc::Rc::clone(&store),
                },
            });
            sides.push((left, right));
        }
        let ones = vec![1.0; sides.len()];
        let lefts: Vec<&[f32]> = sides.iter().map(|(l, _)| l.as_slice()).collect();
        let rights: Vec<&[f32]> = sides.iter().map(|(_, r)| r.as_slice()).collect();
        let want = interleaved(&dsp::mix(&lefts, &ones), &dsp::mix(&rights, &ones));
        let shown = crate::lanes::Shown {
            sample_rate: 44_100,
            frames,
            lanes,
            mix_overview: None,
        };
        let (input, from_storage) = crate::separation_input(&shown, shown.audio());
        spy();
        let before = LIVE.with(Cell::get);
        PEAK.with(|peak| peak.set(before));
        let written = write_mix(id, input).await;
        let peak = (PEAK.with(Cell::get) - before) as f64 / 1e6;
        let record = stop_spy();
        let got = read(&mix_path(id)).await;
        st_remove(ROOT).await;
        written.expect("written");
        assert!(from_storage, "the lanes are read from storage");
        let blocks = frames.div_ceil(MIX_BLOCK_FRAMES);
        let whole: Vec<&str> = record.split(' ').collect();
        assert_eq!(whole.len(), blocks + 1, "the blocks, then meta: {record}");
        assert!(whole.iter().all(|w| *w == "whole"), "{record}");
        assert!(got.as_deref() == Some(&want[..]), "the mix of the lanes");
        assert!(
            peak < 4.0,
            "MEM-4: the heap peaked {peak:.1} MB above the start"
        );
    }

    /// The memory path's input of `song` in memory, as `write_mix` takes it.
    fn input_of(song: &Song) -> Box<dyn engine::mixdown::Pcm> {
        let shown = crate::lanes::Shown::in_memory(song.clone());
        let audio = shown.audio();
        crate::separation_input(&shown, audio).0
    }

    /// MEM-5: each block of the mix, and `meta`, reach storage as a whole
    /// array.
    #[wasm_bindgen_test]
    async fn the_mix_is_written_in_whole_blocks() {
        st_remove(ROOT).await;
        let id = "1792640000010";
        plant_session(id, &[]).await;
        spy();
        let written = write_mix(id, input_of(&one_lane(MIX_BLOCK_FRAMES + 1000))).await;
        let record = stop_spy();
        written.expect("written");
        assert_eq!(
            record, "whole whole whole",
            "two blocks of the mix, then meta"
        );
        st_remove(ROOT).await;
    }

    /// D1 and D3 of M4: the mix file holds the song's one lane, or the mix
    /// of its sources, interleaved, bit for bit, and `meta` names it.
    #[wasm_bindgen_test]
    async fn a_stored_mix_holds_the_song_interleaved() {
        st_remove(ROOT).await;
        let frames = MIX_BLOCK_FRAMES + 1000;
        let lane = one_lane(frames);
        let sources = three_sources(frames);
        let (mix_left, mix_right) = crate::mix_of_sources(&sources);
        assert!(
            mix_left.iter().any(|v| v.abs() == 1.0),
            "the mix of the sources clips, so a sum without dsp::mix differs"
        );
        for (id, song, want) in [
            (
                "1792640000011",
                &lane,
                interleaved(&lane.stems[0].left, &lane.stems[0].right),
            ),
            (
                "1792640000012",
                &sources,
                interleaved(&mix_left, &mix_right),
            ),
        ] {
            plant_session(id, &["bass"]).await;
            write_mix(id, input_of(song)).await.expect("written");
            let got = read(&mix_path(id)).await.expect("a mix file");
            assert_eq!(got.len(), 8 * frames, "{id}");
            assert!(got == want, "{id}: the mix file holds the song interleaved");
            assert_eq!(
                read(&format!("{}.part", mix_path(id))).await,
                None,
                "{id}: no .part file is left"
            );
            let meta = Meta::parse(&String::from_utf8_lossy(
                &read(&format!("{ROOT}/{id}/meta")).await.expect("meta"),
            ));
            assert_eq!(meta.mix, Some((frames, "f32le-1".into())), "{id}");
            assert_eq!(meta.stems, ["bass"], "{id}: the other lines stay");
            assert_eq!(stored_mix_frames(id).await, Some(frames), "{id}");
        }
        let gone = write_mix("1792640000013", input_of(&lane)).await;
        assert!(gone.is_err(), "no session folder");
        assert_eq!(
            st_exists(&format!("{ROOT}/1792640000013")).await.as_bool(),
            Some(false),
            "a forgotten session does not come back"
        );
        st_remove(ROOT).await;
    }

    /// Finding 2-3 of review round 2 of M4: a mix write whose stream
    /// cannot open leaves no `.part` file.
    #[wasm_bindgen_test]
    async fn a_refused_mix_stream_leaves_no_part_file() {
        st_remove(ROOT).await;
        let id = "1792640000030";
        plant_session(id, &[]).await;
        refuse_next_writable();
        let written = write_mix(id, input_of(&one_lane(1000))).await;
        let error = written.expect_err("refused").to_string();
        let part = file_size(&format!("{}.part", mix_path(id))).await;
        st_remove(ROOT).await;
        assert!(error.contains("NoModificationAllowedError"), "{error}");
        assert_eq!(part, None, "no .part file is left");
    }

    /// MEM-1, MEM-4: the mix of a 4-minute song, of three sources or of
    /// one lane, is written block by block.  A whole mix would add about
    /// 85 MB.  The lanes share one buffer, which the test keeps alive.
    #[wasm_bindgen_test]
    async fn the_mix_write_makes_no_song_length_buffer() {
        st_remove(ROOT).await;
        let frames = 4 * 60 * 44_100;
        let wave: Samples = (0..frames)
            .map(|i| 0.3 * (0.01 * i as f32).sin())
            .collect::<Vec<f32>>()
            .into();
        let lane = |name: &str| Stem {
            name: name.into(),
            left: wave.clone(),
            right: wave.clone(),
        };
        let songs = [
            (
                "three sources",
                vec![lane("bass"), lane("drums"), lane("vocals")],
            ),
            ("one lane", vec![lane("acme")]),
        ];
        let mut results = Vec::new();
        for (k, (case, stems)) in songs.into_iter().enumerate() {
            let id = format!("179264000002{k}");
            plant_session(&id, &[]).await;
            let song = Song {
                sample_rate: 44_100,
                frames,
                stems,
            };
            let before = crate::separator::page_wasm_mb();
            write_mix(&id, input_of(&song)).await.expect("written");
            let grown = crate::separator::page_wasm_mb() - before;
            let size = file_size(&mix_path(&id)).await;
            results.push((case, grown, size));
        }
        drop(wave);
        st_remove(ROOT).await;
        for (case, grown, size) in results {
            assert_eq!(
                size,
                Some(8.0 * frames as f64),
                "{case}: the whole mix is written"
            );
            assert!(
                grown < 4.0,
                "MEM-4, {case}: the page grew by {grown} MB while it wrote the mix"
            );
        }
    }

    #[wasm_bindgen_test]
    async fn a_write_never_brings_back_a_removed_folder() {
        write("sessions/1/meta", b"name\tacme\n")
            .await
            .expect("written");
        st_remove(ROOT).await;
        assert!(st_list(ROOT).await.unchecked_into::<Array>().length() == 0);
        let refused = write_bytes("sessions/42", "stems/bass.wav", &[1, 2, 3]).await;
        assert_eq!(
            refused.expect_err("refused").to_string(),
            "sessions/42 is gone (forgotten?)"
        );
        assert!(read("sessions/42/stems/bass.wav").await.is_none());
        assert!(st_list(ROOT).await.unchecked_into::<Array>().length() == 0);

        write("sessions/42/meta", b"name\tacme\n")
            .await
            .expect("written");
        write_bytes("sessions/42", "stems/bass.wav", &[1, 2, 3])
            .await
            .expect("written");
        assert_eq!(
            read("sessions/42/stems/bass.wav").await,
            Some(vec![1, 2, 3])
        );
        st_remove(ROOT).await;
    }

    #[wasm_bindgen(inline_js = r#"
async function store_folder(path, create) {
  let d = await navigator.storage.getDirectory();
  for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p, { create });
  return d;
}
// A session file of `frames` frames of two tones, made in JS, so the test
// page's WebAssembly memory does not hold it.  `mono`: both sides alike.
export async function store_plant_tone(path, frames, mono) {
  const i = path.lastIndexOf("/");
  const d = await store_folder(path.slice(0, i), true);
  const w = await (await d.getFileHandle(path.slice(i + 1), { create: true })).createWritable();
  const block = 131072;
  for (let at = 0; at < frames; at += block) {
    const n = Math.min(block, frames - at);
    const b = new Float32Array(2 * n);
    for (let k = 0; k < n; k++) {
      b[2 * k] = 0.5 * Math.sin(0.01 * (at + k));
      b[2 * k + 1] = mono ? b[2 * k] : 0.25 * Math.sin(0.03 * (at + k));
    }
    await w.write(b);
  }
  await w.close();
}
// A 16-bit stereo WAV at 44.1 kHz of `frames` frames of a tone, made in
// JS, so the test page's WebAssembly memory does not hold it.
export async function store_plant_wav(path, frames) {
  const i = path.lastIndexOf("/");
  const d = await store_folder(path.slice(0, i), true);
  const w = await (await d.getFileHandle(path.slice(i + 1), { create: true })).createWritable();
  const head = new DataView(new ArrayBuffer(44));
  const text = (at, s) => { for (let k = 0; k < 4; k++) head.setUint8(at + k, s.charCodeAt(k)); };
  text(0, "RIFF"); head.setUint32(4, 36 + 4 * frames, true); text(8, "WAVE");
  text(12, "fmt "); head.setUint32(16, 16, true); head.setUint16(20, 1, true);
  head.setUint16(22, 2, true); head.setUint32(24, 44100, true); head.setUint32(28, 4 * 44100, true);
  head.setUint16(32, 4, true); head.setUint16(34, 16, true);
  text(36, "data"); head.setUint32(40, 4 * frames, true);
  await w.write(new Uint8Array(head.buffer));
  const block = 131072;
  for (let at = 0; at < frames; at += block) {
    const n = Math.min(block, frames - at);
    const b = new Int16Array(2 * n);
    for (let k = 0; k < n; k++) {
      b[2 * k] = Math.round(8000 * Math.sin(0.02 * (at + k)));
      b[2 * k + 1] = -b[2 * k];
    }
    await w.write(b);
  }
  await w.close();
}
// Records the bytes of each `Blob.slice` and `Blob.arrayBuffer`.
let store_reads = null;
let store_real = null;
export function store_start_read_spy() {
  const proto = Blob.prototype;
  store_real = [proto.slice, proto.arrayBuffer];
  store_reads = [];
  proto.slice = function (start, end) {
    const s = store_real[0].call(this, start, end);
    store_reads.push(s.size);
    return s;
  };
  proto.arrayBuffer = function () {
    store_reads.push(this.size);
    return store_real[1].call(this);
  };
}
export function store_stop_read_spy() {
  [Blob.prototype.slice, Blob.prototype.arrayBuffer] = store_real;
  return store_reads;
}
export function store_sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
"#)]
    extern "C" {
        async fn store_plant_tone(path: &str, frames: u32, mono: bool);
        async fn store_plant_wav(path: &str, frames: u32);
        fn store_start_read_spy();
        fn store_stop_read_spy() -> Vec<f64>;
        async fn store_sleep(ms: u32);
    }

    /// The sources of a session of `names`, stored as `0.wav`, `1.wav`...
    fn sources_of(names: &[&str]) -> Vec<PlannedSource> {
        let meta = Meta {
            files: names
                .iter()
                .enumerate()
                .map(|(i, n)| (format!("{n}.wav"), format!("{i}.wav")))
                .collect(),
            ..Meta::default()
        };
        crate::store_meta::planned_sources(&meta).expect("planned")
    }

    /// Plants each part file of a decode of `frames` frames of `sources`.
    async fn plant_parts(id: &str, frames: usize, sources: &[PlannedSource], new_mix: bool) {
        for (file, size) in crate::store_meta::session_files(frames, sources) {
            if file == MIX_FILE && !new_mix {
                continue;
            }
            let path = format!("{}/{file}{DECODE_PART}", session_folder(id));
            write(&path, &vec![7; size as usize])
                .await
                .expect("planted");
        }
    }

    /// The part files left in session `id`.
    async fn parts_left(id: &str, frames: usize, sources: &[PlannedSource]) -> Vec<String> {
        let mut left = Vec::new();
        for (file, _) in crate::store_meta::session_files(frames, sources) {
            let path = format!("{}/{file}{DECODE_PART}", session_folder(id));
            if file_size(&path).await.is_some() {
                left.push(file);
            }
        }
        left
    }

    /// Writes `meta` with a usable `mix` line of `frames` for session `id`.
    async fn plant_mix_line(id: &str, frames: usize) {
        let mut meta = plant_session(id, &[]).await;
        meta.mix = Some((frames, MIX_FORMAT.into()));
        write(&format!("{ROOT}/{id}/meta"), meta.to_text().as_bytes())
            .await
            .expect("planted");
    }

    /// D7: the part files move onto their names, and `meta` gets the
    /// `decoded` line and the `mix` line of the new mix.
    #[wasm_bindgen_test]
    async fn a_decode_is_kept_with_its_decoded_and_mix_lines() {
        st_remove(ROOT).await;
        let id = "1792640000040";
        plant_session(id, &["bass"]).await;
        let sources = sources_of(&["drums", "classic drums"]);
        let frames = 5_000;
        plant_parts(id, frames, &sources, true).await;
        let kept = keep_decoded(id, &sources, frames, true, &[2, 1]).await;
        assert_eq!(kept.expect("kept"), Kept::Complete);
        assert_eq!(parts_left(id, frames, &sources).await, Vec::<String>::new());
        let meta = meta_now(id).await.expect("meta");
        assert_eq!(meta.mix, Some((frames, MIX_FORMAT.into())));
        assert_eq!(
            meta.decoded,
            Some(Decoded {
                frames,
                format: MIX_FORMAT.into(),
                channels: vec![(0, 2), (1, 1)],
            })
        );
        assert_eq!(meta.stems, ["bass"], "the other lines stay");
        assert_eq!(file_size(&mix_path(id)).await, Some(8.0 * frames as f64));
        st_remove(ROOT).await;
    }

    /// D8: with a `mix` line now, or a separation or a mix write naming the
    /// session, D7 removes the worker's mix, and `mix.f32` does not change.
    /// With no usable mix, the session stays incomplete.
    #[wasm_bindgen_test]
    async fn a_mix_in_use_is_not_replaced() {
        st_remove(ROOT).await;
        let sources = sources_of(&["drums"]);
        let frames = 3_000;
        let kept_mix = vec![3u8; 8 * frames];
        // A separation names the session, which has no usable mix: the
        // page reopened it while the separation writes the mix.
        let id = "1792640000041";
        plant_session(id, &[]).await;
        write(&mix_path(id), &kept_mix).await.expect("planted");
        plant_parts(id, frames, &sources, true).await;
        set_separating(Some(id.into()));
        let kept = keep_decoded(id, &sources, frames, true, &[2]).await;
        set_separating(None);
        assert_eq!(kept.expect("no error"), Kept::MixBusy);
        assert_eq!(
            read(&mix_path(id)).await,
            Some(kept_mix.clone()),
            "the mix stays"
        );
        assert_eq!(parts_left(id, frames, &sources).await, Vec::<String>::new());
        assert_eq!(
            meta_now(id).await.expect("meta").decoded,
            None,
            "incomplete"
        );
        // A usable `mix` line now: the mix stays, and the session is
        // complete with it.
        let id = "1792640000042";
        plant_mix_line(id, frames).await;
        write(&mix_path(id), &kept_mix).await.expect("planted");
        plant_parts(id, frames, &sources, true).await;
        let kept = keep_decoded(id, &sources, frames, true, &[2]).await;
        assert_eq!(kept.expect("kept"), Kept::Complete);
        assert_eq!(read(&mix_path(id)).await, Some(kept_mix), "the mix stays");
        assert_eq!(parts_left(id, frames, &sources).await, Vec::<String>::new());
        let meta = meta_now(id).await.expect("meta");
        assert_eq!(meta.decoded.map(|d| d.frames), Some(frames));
        assert_eq!(meta.mix, Some((frames, MIX_FORMAT.into())));
        st_remove(ROOT).await;
    }

    /// Waits up to `ms` for `done`: whether it was set.
    async fn set_within(ms: u32, done: &Cell<bool>) -> bool {
        let mut waited = 0;
        while !done.get() && waited < ms {
            store_sleep(20).await;
            waited += 20;
        }
        done.get()
    }

    /// D8a: D7 runs while the decode's guard holds the session, so it must
    /// not wait for the session's saves.  With a planted `mix` line, the
    /// keep path ends.
    #[wasm_bindgen_test]
    async fn the_keep_path_does_not_wait_for_its_own_guard() {
        st_remove(ROOT).await;
        let id = "1792640000043";
        let frames = 2_000;
        plant_mix_line(id, frames).await;
        write(&mix_path(id), &vec![1; 8 * frames])
            .await
            .expect("planted");
        let sources = sources_of(&["drums"]);
        plant_parts(id, frames, &sources, true).await;
        let guard = DecodeGuard::new(id);
        let done = std::rc::Rc::new(Cell::new(false));
        let flag = done.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let sources = sources_of(&["drums"]);
            let kept = keep_decoded("1792640000043", &sources, 2_000, true, &[2]).await;
            flag.set(kept.is_ok());
        });
        let ended = set_within(3_000, &done).await;
        drop(guard);
        st_remove(ROOT).await;
        assert!(ended, "D7 under its own guard ends");
    }

    /// D9: `forget` waits for a decode of the session to end.
    #[wasm_bindgen_test]
    async fn forget_waits_for_a_decode() {
        st_remove(ROOT).await;
        let id = "1792640000044";
        plant_session(id, &[]).await;
        let guard = DecodeGuard::new(id);
        let forgotten = std::rc::Rc::new(Cell::new(false));
        let flag = forgotten.clone();
        wasm_bindgen_futures::spawn_local(async move {
            flag.set(forget("1792640000044").await);
        });
        store_sleep(300).await;
        let during = forgotten.get();
        let still_there = read(&format!("{ROOT}/{id}/meta")).await.is_some();
        drop(guard);
        let ended = set_within(3_000, &forgotten).await;
        st_remove(ROOT).await;
        assert!(!during && still_there, "forget waits while the decode runs");
        assert!(ended, "forget ends once the decode ends");
    }

    /// Finding 1-5 of review round 1: two writers of `meta` on one session
    /// at once.  The page moves a separation's stems while a decode of the
    /// same session ends (the Storage menu opens a session during a
    /// separation).  Each read-modify-write of `meta` waits for the other,
    /// so both the stem lines and the `decoded` line stay.
    #[wasm_bindgen_test]
    async fn two_writers_of_meta_keep_each_others_lines() {
        st_remove(ROOT).await;
        let id = "1792640000070";
        plant_session(id, &[]).await;
        let frames = 2_000;
        let sources = sources_of(&["drums"]);
        plant_parts(id, frames, &sources, true).await;
        write(&format!("{ROOT}/{id}/stems/bass.wav.part"), &[1, 2])
            .await
            .expect("planted");
        let stems_done = std::rc::Rc::new(Cell::new(false));
        let flag = stems_done.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let stored = keep_stored_stems("1792640000070", &["bass".to_string()]).await;
            flag.set(stored.kept.is_ok());
        });
        let kept = keep_decoded(id, &sources, frames, true, &[2]).await;
        let stems_kept = set_within(3_000, &stems_done).await;
        let meta = meta_now(id).await.expect("meta");
        st_remove(ROOT).await;
        assert_eq!(kept.expect("kept"), Kept::Complete);
        assert!(stems_kept, "the stems are kept");
        assert_eq!(meta.stems, ["bass"], "the stem lines stay");
        assert_eq!(
            meta.decoded.map(|d| d.frames),
            Some(frames),
            "the decoded line stays"
        );
    }

    /// Finding 2-15 of review round 2: `save_stems` writes `meta` under its
    /// lock too, so a decode that ends while the stems are saved keeps
    /// the stem lines, and the save keeps the `decoded` line.
    #[wasm_bindgen_test]
    async fn saved_stems_and_a_kept_decode_keep_each_others_lines() {
        for delay in [0, 5, 20] {
            st_remove(ROOT).await;
            let id = "1792640000072";
            plant_session(id, &[]).await;
            let frames = 2_000;
            let sources = sources_of(&["drums"]);
            plant_parts(id, frames, &sources, true).await;
            let saved = std::rc::Rc::new(Cell::new(false));
            let flag = saved.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let stem = Stem {
                    name: "bass".into(),
                    left: vec![0.25; 4_000].into(),
                    right: vec![-0.25; 4_000].into(),
                };
                flag.set(save_stems("1792640000072", 44_100, &[stem]).await.is_ok());
            });
            st_sleep(delay).await;
            let kept = keep_decoded(id, &sources, frames, true, &[2]).await;
            let stems_saved = set_within(3_000, &saved).await;
            let meta = meta_now(id).await.expect("meta");
            st_remove(ROOT).await;
            assert_eq!(kept.expect("kept"), Kept::Complete, "delay {delay}");
            assert!(stems_saved, "delay {delay}: the stems are saved");
            assert_eq!(meta.stems, ["bass"], "delay {delay}: the stem lines stay");
            assert_eq!(
                meta.decoded.map(|d| d.frames),
                Some(frames),
                "delay {delay}: the decoded line stays"
            );
        }
    }

    /// Finding 1-22 of review round 1: a `mix` line whose file has the
    /// wrong size is no usable mix, so D7 moves the worker's mix into its
    /// place, and the session heals.
    #[wasm_bindgen_test]
    async fn a_mix_line_with_a_wrong_size_file_is_replaced() {
        st_remove(ROOT).await;
        let id = "1792640000071";
        let frames = 2_000;
        plant_mix_line(id, frames).await;
        write(&mix_path(id), &vec![1; 8 * frames - 8])
            .await
            .expect("planted");
        let sources = sources_of(&["drums"]);
        plant_parts(id, frames, &sources, true).await;
        let kept = keep_decoded(id, &sources, frames, true, &[2]).await;
        let mix = read(&mix_path(id)).await;
        st_remove(ROOT).await;
        assert_eq!(kept.expect("kept"), Kept::Complete);
        assert_eq!(mix, Some(vec![7; 8 * frames]), "the worker's mix");
    }
}
