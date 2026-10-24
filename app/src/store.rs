//! Song sessions kept in the browser's private file system (OPFS), so a
//! reload brings back the song and its separated stems without running the
//! network again.  Web only.
//!
//! Layout under the OPFS root:
//! * `sessions/current`: the id of the session shown last (text).
//! * `sessions/<id>/meta`: tab-separated lines: `name`, `rate`, one `file`
//!   line per source file (original name, stored name) and one `stem` line
//!   per separated stem.
//! * `sessions/<id>/src/<n>.<ext>`: the files as dropped.
//! * `sessions/<id>/stems/<stem>.wav`: separated stems, 16-bit PCM.
//!
//! Ids are creation times in milliseconds, made unique within the page: a
//! second session in the same millisecond takes the next one.  Every
//! dropped song gets its own session; the Storage menu lists them, opens
//! one or forgets it.

use crate::store_meta::Meta;
use anyhow::{Result, anyhow};
use engine::{Samples, Song, Stem};
use js_sys::{Array, Uint8Array};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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
  const url = URL.createObjectURL(new Blob([bytes], { type: mime }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60000);
}
export function st_sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
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
    async fn st_usage() -> JsValue;
    async fn st_list(path: &str) -> JsValue;
    /// Whether the folder `path` exists (true also when the browser cannot
    /// tell).
    async fn st_exists(path: &str) -> JsValue;
    fn save_file(name: &str, bytes: &Uint8Array, mime: &str);
    async fn st_sleep(ms: u32) -> JsValue;
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

/// A stem name as a file name: letters, digits, spaces and dashes only.
fn file_stem(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
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

/// Adds separated stems to session `id`.
pub async fn save_stems(id: &str, rate: u32, stems: &[Stem]) -> Result<()> {
    SAVING.with_borrow_mut(|s| *s.entry(id.to_string()).or_default() += 1);
    let saved = save_stems_now(id, rate, stems).await;
    SAVING.with_borrow_mut(|s| {
        if let Some(n) = s.get_mut(id) {
            *n -= 1;
            if *n == 0 {
                s.remove(id);
            }
        }
    });
    saved
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
            &format!("stems/{}.wav", file_stem(&s.name)),
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
        let bytes = read(&format!("{base}/stems/{}.wav", file_stem(name)))
            .await
            .ok_or_else(|| anyhow!("stem {name} is missing from browser storage"))?;
        let audio = engine::decode::decode_bytes(bytes, Some("wav"))?;
        let mut ch = audio.channels.into_iter();
        let left: Samples = ch.next().unwrap_or_default().into();
        let right = ch.next().map_or_else(|| left.clone(), Samples::from);
        stems.push(Stem {
            name: name.clone(),
            left,
            right,
        });
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

/// Browser tests of the private write path (`cargo run -p xtask --
/// web-test`); the public functions are tested in `tests/web.rs`.
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

    /// A stereo tone as a WAV file of `frames` frames at 8 kHz.
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
