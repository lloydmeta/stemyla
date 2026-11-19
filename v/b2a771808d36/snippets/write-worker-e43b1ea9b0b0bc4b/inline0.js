
async function folder(path, create, from) {
  let d = from || await navigator.storage.getDirectory();
  for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p, { create });
  return d;
}
function split(path) {
  const i = path.lastIndexOf("/");
  return i < 0 ? ["", path] : [path.slice(0, i), path.slice(i + 1)];
}
// Safari 27 writes a typed array view's whole buffer, not just the view,
// so only whole arrays are written: a view is copied out first (MEM-5).
function whole(bytes) {
  return bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength ? bytes : bytes.slice();
}
// The handle of the file `within/rel`, with the folders of `rel` made, as
// `st_write` of the page makes them; null when the folder `within` is gone.
async function file_of(within, rel) {
  let base;
  try { base = await folder(within, false); }
  catch (e) { if (within && e.name === "NotFoundError") return null; throw e; }
  const [d, n] = split("/" + rel);
  return await (await folder(d, true, base)).getFileHandle(n, { create: true });
}
// `write` returns the bytes written.  Old browsers return a promise, so
// it is awaited, as `truncate`, `flush` and `close` are.
async function put(h, bytes, at) {
  const n = await h.write(whole(bytes), { at });
  if (n !== bytes.byteLength) throw new Error(`wrote ${n} of ${bytes.byteLength} bytes`);
}
// Opens a sync access handle of `within/rel`, cuts the file to 0 bytes
// if `cut`, runs `body` with it, flushes and closes it, also on a failure.
async function with_handle(within, rel, cut, body) {
  try {
    const fh = await file_of(within, rel);
    if (!fh) return "gone";
    const h = await fh.createSyncAccessHandle();
    try {
      if (cut) await h.truncate(0);
      await body(h);
      await h.flush();
    } finally {
      await h.close();
    }
    return "written";
  } catch (e) {
    return (e && e.name) || "Error";
  }
}
// A `Blob` is read one slice at a time: the worker references at most one
// slice of 8 MiB at a time, until the garbage collector frees it (TD-11).
const SLICE = 8 << 20;
export async function write_whole(within, rel, body) {
  return with_handle(within, rel, true, async (h) => {
    if (!(body instanceof Blob)) return put(h, body, 0);
    for (let at = 0; at < body.size; at += SLICE) {
      const end = Math.min(at + SLICE, body.size);
      await put(h, new Uint8Array(await body.slice(at, end).arrayBuffer()), at);
    }
  });
}
export async function write_block(within, rel, at, bytes) {
  return with_handle(within, rel, at === 0, (h) => put(h, bytes, at));
}
export async function write_end(within, rel, size) {
  try {
    let base;
    try { base = await folder(within, false); }
    catch (e) { if (within && e.name === "NotFoundError") return "gone"; throw e; }
    const [d, n] = split("/" + rel);
    let dir;
    try { dir = await folder(d, false, base); }
    catch (e) { if (size === null && e.name === "NotFoundError") return "written"; throw e; }
    if (size === null) {
      try { await dir.removeEntry(n); }
      catch (e) { if (e.name !== "NotFoundError") throw e; }
      return "written";
    }
    const held = (await (await dir.getFileHandle(n)).getFile()).size;
    return held === size ? "written" : `the file holds ${held} of ${size} bytes`;
  } catch (e) {
    return (e && e.name) || "Error";
  }
}
