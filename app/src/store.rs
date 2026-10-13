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
use engine::{Song, Stem};
use js_sys::{Array, Uint8Array};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
async function dir(path, create) {
  let d = await navigator.storage.getDirectory();
  for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p, { create });
  return d;
}
function split(path) {
  const i = path.lastIndexOf("/");
  return [path.slice(0, i), path.slice(i + 1)];
}
export async function st_write(path, bytes) {
  try {
    const [d, n] = split(path);
    const fh = await (await dir(d, true)).getFileHandle(n, { create: true });
    const w = await fh.createWritable();
    await w.write(bytes);
    await w.close();
    return true;
  } catch (e) { console.warn("storage write failed", path, e); return false; }
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
  } catch (e) { return false; }
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
"#)]
extern "C" {
    async fn st_write(path: &str, bytes: &Uint8Array) -> JsValue;
    async fn st_read(path: &str) -> JsValue;
    async fn st_remove(path: &str) -> JsValue;
    async fn st_usage() -> JsValue;
    async fn st_list(path: &str) -> JsValue;
    fn save_file(name: &str, bytes: &Uint8Array, mime: &str);
}

const ROOT: &str = "sessions";

async fn write(path: &str, bytes: &[u8]) -> Result<()> {
    if st_write(path, &Uint8Array::from(bytes)).await.as_bool() == Some(true) {
        Ok(())
    } else {
        Err(anyhow!(
            "browser storage refused {path} (full, or not available)"
        ))
    }
}

async fn read(path: &str) -> Option<Vec<u8>> {
    let v = st_read(path).await;
    if v.is_null() {
        None
    } else {
        Some(v.unchecked_into::<Uint8Array>().to_vec())
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
    let base = format!("{ROOT}/{id}");
    let mut meta = Meta::parse(&String::from_utf8_lossy(
        &read(&format!("{base}/meta"))
            .await
            .ok_or_else(|| anyhow!("session {id} is gone"))?,
    ));
    meta.stems.clear();
    for s in stems {
        write(
            &format!("{base}/stems/{}.wav", file_stem(&s.name)),
            &engine::wav::encode_pcm16(&s.left, &s.right, rate),
        )
        .await?;
        meta.stems.push(s.name.clone());
    }
    write(&format!("{base}/meta"), meta.to_text().as_bytes()).await
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

/// Deletes session `id` (song and stems).
pub async fn forget(id: &str) {
    st_remove(&format!("{ROOT}/{id}")).await;
    if current().await.as_deref() == Some(id) {
        st_remove(&format!("{ROOT}/current")).await;
    }
}

/// The current session as a song: the dropped files, plus the separated
/// stems and the original mix when there are stems.  `None` when nothing is
/// stored.
pub async fn restore() -> Option<Result<(String, Song, Meta)>> {
    restore_id(&current().await?).await
}

/// Session `id` as a song, made current.
pub async fn restore_id(id: &str) -> Option<Result<(String, Song, Meta)>> {
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
        let left = ch.next().unwrap_or_default();
        let right = ch.next().unwrap_or_else(|| left.clone());
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
