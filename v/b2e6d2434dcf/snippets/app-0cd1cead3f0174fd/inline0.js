
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
// Removes each file in the folder `path` whose `lastModified` is before
// `before` (ms since 1970); the names removed, sorted, joined by spaces.
export async function st_remove_older(path, before) {
  let d;
  try { d = await dir(path, false); } catch (e) { return ""; }
  const gone = [];
  for await (const [name, h] of d.entries()) {
    if (h.kind !== "file") continue;
    try {
      if ((await h.getFile()).lastModified < before) { await d.removeEntry(name); gone.push(name); }
    } catch (e) { console.warn("storage: could not remove an old file", path, name, e); }
  }
  return gone.sort().join(" ");
}
// Offers `file` as a download of type `mime`.  The `Blob` refers to the
// file: it copies no bytes (an OPFS `File` may have no type).
export function st_offer(name, file, mime) {
  save_blob(name, new Blob([file], { type: mime }));
}
// The page's start, in whole ms since 1970.
export function st_page_ms() { return Math.floor(performance.timeOrigin); }
// Bytes `start..end` of a `File`: one read of a slice, never the file.
export async function st_slice(file, start, end) {
  return new Uint8Array(await file.slice(start, end).arrayBuffer());
}
