
export async function cache_get(name) {
  try {
    const root = await navigator.storage.getDirectory();
    const fh = await root.getFileHandle(name);
    const f = await fh.getFile();
    return new Uint8Array(await f.arrayBuffer());
  } catch (e) { return null; }
}
export async function cache_put(name, bytes) {
  try {
    const root = await navigator.storage.getDirectory();
    const fh = await root.getFileHandle(name, { create: true });
    const w = await fh.createWritable();
    await w.write(bytes);
    await w.close();
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
export async function download_to_opfs(url, name, progress) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`HTTP ${r.status} from ${url}`);
  const total = Number(r.headers.get("content-length")) || 0;
  const root = await navigator.storage.getDirectory();
  const fh = await root.getFileHandle(name, { create: true });
  const w = await fh.createWritable();
  const reader = r.body.getReader();
  let done = 0, last = 0;
  for (;;) {
    const { value, done: end } = await reader.read();
    if (end) break;
    await w.write(value);
    done += value.length;
    if (done - last >= 4e6) { progress(done, total); last = done; }
  }
  await w.close();
  progress(done, total);
  return done;
}
export async function opfs_remove(name) {
  try { const root = await navigator.storage.getDirectory(); await root.removeEntry(name); return true; } catch (e) { return false; }
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
