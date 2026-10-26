//! Song sessions kept in the browser's private file system (OPFS), so a
//! reload brings back the song and its separated stems without running the
//! network again.  Web only.
//!
//! Layout under the OPFS root:
//! * `sessions/current`: the id of the session shown last (text).
//! * `sessions/<id>/meta`: tab-separated lines: `name`, `rate`, one `file`
//!   line per source file (original name, stored name), one `stem` line
//!   per separated stem, and a `mix` line (frames, format) when `mix.f32`
//!   is complete.
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
//! On the storage path, the page writes the mix file before it posts, the
//! worker reads it and writes the stem files, and the page moves them into
//! place and writes `meta`.  On the memory path, the page
//! writes the stems itself ([`save_stems`]).
//!
//! Ids are creation times in milliseconds, made unique within the page: a
//! second session in the same millisecond takes the next one.  Every
//! dropped song gets its own session; the Storage menu lists them, opens
//! one or forgets it.

use crate::store_meta::{MIX_FORMAT, Meta};
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

/// Starts a session for files the user dropped and makes it current.
/// Returns its id.  Each file is written as the browser's `Blob`, so the
/// page keeps no copy of its bytes to save it (MEM-1).  A save that fails
/// removes what it wrote: the caller gets no id to use, and a session
/// without `meta` is not listed, so it cannot be forgotten.
pub async fn save_sources(files: &[(String, web_sys::Blob)], rate: u32) -> Result<String> {
    let id = new_id().await;
    let base = format!("{ROOT}/{id}");
    let saved = save_sources_now(&id, &base, files, rate).await;
    if saved.is_err() && st_remove(&base).await.as_bool() != Some(true) {
        log::warn!("could not remove {base} after a failed save");
    }
    saved.map(|()| id)
}

async fn save_sources_now(
    id: &str,
    base: &str,
    files: &[(String, web_sys::Blob)],
    rate: u32,
) -> Result<()> {
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
    write(&format!("{base}/meta"), meta.to_text().as_bytes()).await?;
    write(&format!("{ROOT}/current"), id.as_bytes()).await
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

/// Writes the mix of `song` to session `id` as its mix file, in blocks of
/// [`MIX_BLOCK_FRAMES`], each a new whole array (D3 of M4, MEM-5).  A song
/// of one source lane writes that lane.  A song of several sources writes
/// the mix of each block, so the page never makes a song-length mix.  It
/// first removes the `mix` line and any old mix file, and it writes the
/// `mix` line only for a complete file.  It never makes a forgotten
/// session folder again.
pub async fn write_mix(id: &str, song: &Song) -> Result<()> {
    saving(id, async {
        if CLOSED.get() {
            return Err(anyhow!("browser storage is being reset"));
        }
        WRITES.set(WRITES.get() + 1);
        let written = write_mix_now(id, song).await;
        WRITES.set(WRITES.get() - 1);
        written
    })
    .await
}

async fn write_mix_now(id: &str, song: &Song) -> Result<()> {
    let base = session_folder(id);
    let mut meta = Meta::parse(&String::from_utf8_lossy(
        &read(&format!("{base}/meta"))
            .await
            .ok_or_else(|| anyhow!("session {id} is gone"))?,
    ));
    if meta.mix.take().is_some() {
        write_bytes(&base, "meta", meta.to_text().as_bytes()).await?;
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
    if let Err(e) = put_mix_blocks(&w, song).await {
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
    meta.mix = Some((song.frames, MIX_FORMAT.into()));
    if let Err(e) = write_bytes(&base, "meta", meta.to_text().as_bytes()).await {
        st_remove(&mix_path(id)).await;
        return Err(e);
    }
    Ok(())
}

/// Writes the mix of `song` to the stream `w`, one block at a time.
async fn put_mix_blocks(w: &JsValue, song: &Song) -> Result<()> {
    let lane = crate::original_lane(song);
    let sources: Vec<&Stem> = song
        .stems
        .iter()
        .filter(|s| !crate::DERIVED.contains(&s.name.as_str()))
        .collect();
    let lefts: Vec<&[f32]> = sources.iter().map(|s| s.left.as_slice()).collect();
    let rights: Vec<&[f32]> = sources.iter().map(|s| s.right.as_slice()).collect();
    let volumes = vec![1.0; sources.len()];
    let mut start = 0;
    while start < song.frames {
        if CLOSED.get() {
            return Err(anyhow!("browser storage is being reset"));
        }
        let end = (start + MIX_BLOCK_FRAMES).min(song.frames);
        let bytes = match lane {
            Some(lane) => interleave(&lane.left[start..end], &lane.right[start..end]),
            None => interleave(
                &dsp::mix_range(&lefts, &volumes, start, end - start),
                &dsp::mix_range(&rights, &volumes, start, end - start),
            ),
        };
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

/// One block of the mix file: interleaved 32-bit little-endian floats.
fn interleave(left: &[f32], right: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 * left.len());
    for (l, r) in left.iter().zip(right) {
        out.extend_from_slice(&l.to_le_bytes());
        out.extend_from_slice(&r.to_le_bytes());
    }
    out
}

/// Removes the `mix` line of session `id` and its mix file, after the
/// worker could not read it (D6 of M4).  So the next start writes the mix
/// again, and a bad file takes no space.  The removal counts as a save
/// from this call on, before the future first runs, so a start that reads
/// `meta` after this call waits for it ([`stored_mix_frames`]).
pub fn forget_mix(id: &str) -> impl std::future::Future<Output = Result<()>> + 'static {
    let held = SaveHeld::new(id);
    let id = id.to_string();
    async move {
        let _held = held;
        let base = session_folder(&id);
        let mut meta = Meta::parse(&String::from_utf8_lossy(
            &read(&format!("{base}/meta"))
                .await
                .ok_or_else(|| anyhow!("session {id} is gone"))?,
        ));
        if meta.mix.take().is_some() {
            write_bytes(&base, "meta", meta.to_text().as_bytes()).await?;
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
/// `frames`, so the player never reads past a stem's end.
pub async fn read_stem(path: &str, name: &str, frames: usize) -> Result<Stem> {
    let bytes = read(path)
        .await
        .ok_or_else(|| anyhow!("stem {name} is missing from browser storage"))?;
    let audio = engine::decode::decode_bytes(bytes, Some("wav"))?;
    // A session stored before decision 0012 from stems at mixed rates can
    // hold stems one frame shorter than its source now opens with.
    let fitted = |c: Vec<f32>| -> Samples { padded_to(c, frames).into() };
    let mut ch = audio.channels.into_iter();
    let left = fitted(ch.next().unwrap_or_default());
    let right = ch.next().map_or_else(|| left.clone(), fitted);
    Ok(Stem {
        name: name.into(),
        left,
        right,
    })
}

async fn save_stems_now(id: &str, rate: u32, stems: &[Stem]) -> Result<()> {
    let base = format!("{ROOT}/{id}");
    let mut meta = Meta::parse(&String::from_utf8_lossy(
        &read(&format!("{base}/meta"))
            .await
            .ok_or_else(|| anyhow!("session {id} is gone"))?,
    ));
    meta.stems.clear();
    for s in stems {
        write_bytes(
            &base,
            &format!("stems/{}", stem_file(&s.name)),
            &engine::wav::encode_pcm16(&s.left, &s.right, rate),
        )
        .await?;
        meta.stems.push(s.name.clone());
    }
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

/// Session `id` as a song, made current.
pub async fn restore_id(id: &str) -> Option<Result<(String, Song, Meta)>> {
    settled(id).await;
    let id = id.to_string();
    let base = format!("{ROOT}/{id}");
    let meta = Meta::parse(&String::from_utf8_lossy(
        &read(&format!("{base}/meta")).await?,
    ));
    let r = rebuild(&base, &meta)
        .await
        .map(|song| (id.clone(), song, meta));
    if r.is_ok()
        && write(&format!("{ROOT}/current"), id.as_bytes())
            .await
            .is_err()
    {
        log::warn!("could not mark session {id} as current");
    }
    Some(r)
}

async fn rebuild(base: &str, meta: &Meta) -> Result<Song> {
    let mut files = Vec::new();
    for (orig, stored) in &meta.files {
        let bytes = read(&format!("{base}/src/{stored}"))
            .await
            .ok_or_else(|| anyhow!("{orig} is missing from browser storage"))?;
        files.push((orig.clone(), bytes));
    }
    let source = Song::load_files(files)?;
    if meta.stems.is_empty() {
        return Ok(source);
    }
    let mut stems = Vec::new();
    for name in &meta.stems {
        let path = format!("{base}/stems/{}", stem_file(name));
        stems.push(read_stem(&path, name, source.frames).await?);
    }
    Ok(crate::with_original(&source, stems))
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
        assert_eq!(
            record, "same whole whole",
            "the file itself, then meta and current"
        );
        let (got, song, meta) = restore().await.expect("stored").expect("readable");
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
        let stem = read_stem(&path, "bass", 5).await.expect("read");
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

    /// MEM-5: each block of the mix, and `meta`, reach storage as a whole
    /// array.
    #[wasm_bindgen_test]
    async fn the_mix_is_written_in_whole_blocks() {
        st_remove(ROOT).await;
        let id = "1792640000010";
        plant_session(id, &[]).await;
        spy();
        let written = write_mix(id, &one_lane(MIX_BLOCK_FRAMES + 1000)).await;
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
            write_mix(id, song).await.expect("written");
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
        let gone = write_mix("1792640000013", &lane).await;
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
        let written = write_mix(id, &one_lane(1000)).await;
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
            write_mix(&id, &song).await.expect("written");
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
}
