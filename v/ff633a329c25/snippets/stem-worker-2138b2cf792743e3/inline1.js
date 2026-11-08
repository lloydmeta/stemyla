
export async function cache_get(name) {
  const root = await navigator.storage.getDirectory();
  let fh;
  try { fh = await root.getFileHandle(name); }
  catch (e) { if (e.name === "NotFoundError") return null; throw e; }
  const f = await fh.getFile();
  return new Uint8Array(await f.arrayBuffer());
}
export async function cache_file(name) {
  const root = await navigator.storage.getDirectory();
  try { return await (await root.getFileHandle(name)).getFile(); }
  catch (e) { if (e.name === "NotFoundError") return null; throw e; }
}
export function blob_range(blob, start, end) {
  return new Uint8Array(new FileReaderSync().readAsArrayBuffer(blob.slice(start, end)));
}
let queue_tail = Promise.resolve();
export function queue_turn() {
  let release;
  const held = new Promise((r) => { release = r; });
  const turn = queue_tail.then(() => release);
  queue_tail = turn.then(() => held);
  return turn;
}
export function storage_lock(waiting) {
  const name = "stemyla-model-storage";
  return new Promise((granted, failed) => {
    const hold = () => new Promise((release) => granted(release));
    navigator.locks
      .request(name, { ifAvailable: true }, (lock) => {
        if (lock) return hold();
        waiting();
        navigator.locks.request(name, hold).catch(failed);
      })
      .catch(failed);
  });
}
export async function opfs_has(name) {
  try { const root = await navigator.storage.getDirectory(); await root.getFileHandle(name); return true; }
  catch (e) { return e.name !== "NotFoundError"; }
}
// Safari 27 writes a typed array view's whole buffer, not just the view
// (`new Uint8Array(16).subarray(4, 8)` writes 16 bytes), so only whole
// arrays are written: a view is copied out first.
function whole(bytes) {
  return bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength ? bytes : bytes.slice();
}
async function write_file(fh, body) {
  const w = await fh.createWritable();
  try {
    await body(w);
    await w.close();
  } catch (e) {
    try { await w.abort(); } catch (_) {}
    throw e;
  }
}
export async function opfs_write(name, src) {
  try {
    const root = await navigator.storage.getDirectory();
    const fh = await root.getFileHandle(name, { create: true });
    const slice = 8 << 20;
    const size = src instanceof Blob ? src.size : src.length;
    await write_file(fh, async (w) => {
      for (let at = 0; at < size; at += slice) {
        const end = Math.min(at + slice, size);
        await w.write(src instanceof Blob ? src.slice(at, end) : whole(src.subarray(at, end)));
      }
    });
    return true;
  } catch (e) {
    await opfs_remove(name);
    return false;
  }
}
export async function cache_put(name, bytes) {
  const part = name + ".part";
  if (await opfs_write(part, bytes) && await opfs_commit(part, name)) return true;
  await opfs_remove(part);
  return false;
}
export async function opfs_commit(from, to) {
  try {
    const root = await navigator.storage.getDirectory();
    const fh = await root.getFileHandle(from);
    await fh.move(root, to);
    return true;
  } catch (e) { return false; }
}
export async function gpu_info(pref) {
  if (!navigator.gpu) return "no WebGPU";
  const a = await navigator.gpu.requestAdapter({ powerPreference: pref });
  if (!a) return "no WebGPU adapter";
  const i = a.info || {};
  return [i.vendor, i.architecture, i.description].filter((s) => s).join(" ") || "unknown adapter";
}
export async function blob_bytes(blob) {
  return new Uint8Array(await blob.arrayBuffer());
}
export async function download_to_opfs(url, name, progress, follow) {
  const ctl = new AbortController();
  let idle;
  const arm = () => {
    clearTimeout(idle);
    idle = setTimeout(() => ctl.abort(new Error("no data from the server for 60 s")), 60000);
  };
  arm();
  try { return await download_with(url, name, progress, ctl.signal, arm, follow); }
  finally { clearTimeout(idle); }
}
async function download_with(url, name, progress, signal, arm, follow) {
  const r = await fetch(url, { signal, redirect: follow ? "follow" : "error" });
  if (!r.ok) throw new Error(`HTTP ${r.status} from ${url}`);
  const total = Number(r.headers.get("content-length")) || 0;
  const root = await navigator.storage.getDirectory();
  const fh = await root.getFileHandle(name, { create: true });
  const reader = r.body.getReader();
  let done = 0, last = 0;
  try {
    await write_file(fh, async (w) => {
      for (;;) {
        const { value, done: end } = await reader.read();
        if (end) break;
        arm();
        await w.write(whole(value));
        done += value.length;
        if (done - last >= 4e6) { progress(done, total); last = done; }
      }
    });
  } catch (e) {
    reader.cancel().catch(() => {});
    throw e;
  }
  progress(done, total);
  return done;
}
export async function opfs_remove(name) {
  try { const root = await navigator.storage.getDirectory(); await root.removeEntry(name); return true; }
  catch (e) { return e.name === "NotFoundError"; }
}
export async function sha256_hex(bytes) {
  const d = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return Array.from(d, (b) => b.toString(16).padStart(2, "0")).join("");
}
export async function fetch_bytes(url) {
  try {
    const r = await fetch(url);
    if (!r.ok) return null;
    return new Uint8Array(await r.arrayBuffer());
  } catch (e) { return null; }
}
export function perf_now() { return performance.now(); }
