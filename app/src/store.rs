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
//! Ids are creation times in milliseconds.  Every dropped song gets its own
//! session; the Storage menu lists them, opens one or forgets it.

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
  return [path.slice(0, i), path.slice(i + 1)];
}
export async function st_write(within, rel, bytes) {
  try {
    let base;
    try { base = await dir(within, false); }
    catch (e) { if (within && e.name === "NotFoundError") return "gone"; throw e; }
    const [d, n] = split("/" + rel);
    const fh = await (await dir(d, true, base)).getFileHandle(n, { create: true });
    const w = await fh.createWritable();
    await w.write(bytes);
    await w.close();
    return true;
  } catch (e) { console.warn("storage write failed", within, rel, e); return false; }
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
    /// Writes `within/rel`, creating the folders in `rel`; `within` must
    /// exist already.  True, false, or "gone" when `within` does not exist.
    async fn st_write(within: &str, rel: &str, bytes: &Uint8Array) -> JsValue;
    async fn st_read(path: &str) -> JsValue;
    /// Whether `path` is gone (removed, or never there).
    async fn st_remove(path: &str) -> JsValue;
    async fn st_usage() -> JsValue;
    async fn st_list(path: &str) -> JsValue;
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
    write_within("", path, bytes).await
}

/// Writes `within/rel` only if the folder `within` still exists, so a save
/// into a session forgotten meanwhile cannot bring it back.
async fn write_within(within: &str, rel: &str, bytes: &[u8]) -> Result<()> {
    if CLOSED.get() {
        return Err(anyhow!("browser storage is being reset"));
    }
    WRITES.set(WRITES.get() + 1);
    let written = st_write(within, rel, &Uint8Array::from(bytes)).await;
    WRITES.set(WRITES.get() - 1);
    match (written.as_bool(), written.as_string().as_deref()) {
        (Some(true), _) => Ok(()),
        (_, Some("gone")) => Err(anyhow!("{within} is gone (forgotten?)")),
        _ => Err(anyhow!(
            "browser storage refused {} (full, or not available)",
            if within.is_empty() {
                rel.to_string()
            } else {
                format!("{within}/{rel}")
            }
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
            reason = "MEM-1: the decoder reads wasm memory, so a stored file is copied in once"
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
/// Returns its id.
pub async fn save_sources(files: &[(String, Vec<u8>)], rate: u32) -> Result<String> {
    let id = format!("{}", js_sys::Date::now() as u64);
    let base = format!("{ROOT}/{id}");
    let mut meta = Meta {
        rate,
        ..Default::default()
    };
    for (i, (name, bytes)) in files.iter().enumerate() {
        let ext = name
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 5 && *e != name)
            .unwrap_or("bin")
            .to_ascii_lowercase();
        let stored = format!("{i}.{ext}");
        write(&format!("{base}/src/{stored}"), bytes).await?;
        meta.files.push((name.clone(), stored));
    }
    meta.name = files
        .first()
        .map(|(n, _)| crate::store_meta::file_title(n))
        .unwrap_or_default();
    write(&format!("{base}/meta"), meta.to_text().as_bytes()).await?;
    write(&format!("{ROOT}/current"), id.as_bytes()).await?;
    Ok(id)
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
        write_within(
            &base,
            &format!("stems/{}.wav", file_stem(&s.name)),
            &engine::wav::encode_pcm16(&s.left, &s.right, rate),
        )
        .await?;
        meta.stems.push(s.name.clone());
    }
    write_within(&base, "meta", meta.to_text().as_bytes()).await
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
pub fn download(name: &str, bytes: &[u8], mime: &str) {
    save_file(name, &Uint8Array::from(bytes), mime);
}

/// Browser tests of the private write path (`cargo run -p xtask --
/// web-test`); the public functions are tested in `tests/web.rs`.
#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn a_write_never_brings_back_a_removed_folder() {
        st_remove(ROOT).await;
        let refused = write_within("sessions/42", "stems/bass.wav", &[1, 2, 3]).await;
        assert_eq!(
            refused.expect_err("refused").to_string(),
            "sessions/42 is gone (forgotten?)"
        );
        assert!(read("sessions/42/stems/bass.wav").await.is_none());
        assert!(st_list(ROOT).await.unchecked_into::<Array>().length() == 0);

        write("sessions/42/meta", b"name\tacme\n")
            .await
            .expect("written");
        write_within("sessions/42", "stems/bass.wav", &[1, 2, 3])
            .await
            .expect("written");
        assert_eq!(
            read("sessions/42/stems/bass.wav").await,
            Some(vec![1, 2, 3])
        );
        st_remove(ROOT).await;
    }
}
