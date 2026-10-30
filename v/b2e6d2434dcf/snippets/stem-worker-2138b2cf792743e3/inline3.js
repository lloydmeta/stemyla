
async function folder_and_name(path) {
  const parts = path.split("/").filter((s) => s);
  const name = parts.pop();
  let d = await navigator.storage.getDirectory();
  for (const p of parts) d = await d.getDirectoryHandle(p);
  return [d, name];
}
export async function out_open(path) {
  const [d, name] = await folder_and_name(path);
  const h = await (await d.getFileHandle(name, { create: true })).createSyncAccessHandle();
  try { await h.truncate(0); }
  catch (e) { try { await h.close(); } catch (_) {} throw e; }
  return h;
}
// `write` returns the bytes written.  Old browsers return a promise, so
// it is awaited, as `flush` and `close` are.
export async function out_write(h, bytes, at) {
  const n = await h.write(bytes, { at });
  if (n !== bytes.byteLength) throw new Error(`wrote ${n} of ${bytes.byteLength} bytes`);
}
export async function out_close(h) {
  try { await h.flush(); }
  finally { await h.close(); }
}
export async function out_drop(h) {
  try { await h.close(); } catch (_) {}
}
export async function save_remove(path) {
  try { const [d, name] = await folder_and_name(path); await d.removeEntry(name); return true; }
  catch (e) { return e.name === "NotFoundError"; }
}
// The worker's JS heap after a garbage collection, where Chrome gives
// `performance.memory` here and `gc` is exposed; else null.
export function js_heap() {
  const m = globalThis.performance && performance.memory;
  if (!m) return null;
  if (typeof globalThis.gc === "function") globalThis.gc();
  return m.usedJSHeapSize;
}
