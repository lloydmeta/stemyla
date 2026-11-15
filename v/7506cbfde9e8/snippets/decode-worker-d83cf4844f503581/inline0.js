
const files = new Map();
const parts = new Map();
let next_id = 1;
async function folder(path, create) {
  let d = await navigator.storage.getDirectory();
  for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p, { create });
  return d;
}
function split(path) {
  const i = path.lastIndexOf("/");
  return i < 0 ? ["", path] : [path.slice(0, i), path.slice(i + 1)];
}
// The file at `path` as a `File`, kept in the table: [its number, its size].
export async function file_open(path) {
  const [d, n] = split(path);
  const f = await (await (await folder(d, false)).getFileHandle(n)).getFile();
  const id = next_id++;
  files.set(id, f);
  return [id, f.size];
}
// Bytes `start..end` of a kept file, read synchronously (workers only).
export function file_read(id, start, end) {
  const f = files.get(id);
  if (!f) throw new Error(`no open file ${id}`);
  return new Uint8Array(new FileReaderSync().readAsArrayBuffer(f.slice(start, end)));
}
export function files_forget() { files.clear(); }
// An empty file `rel` in the folder `session`, which must exist, so a
// forgotten session does not come back.  The folders in `rel` are made.
export async function part_open(session, rel) {
  const [d, n] = split(rel);
  let dir = await folder(session, false);
  for (const p of d.split("/").filter((s) => s)) dir = await dir.getDirectoryHandle(p, { create: true });
  const h = await (await dir.getFileHandle(n, { create: true })).createSyncAccessHandle();
  try { await h.truncate(0); } catch (e) { await h.close(); throw e; }
  const id = next_id++;
  parts.set(id, h);
  return id;
}
// `write` and `read` are synchronous on a sync access handle, so the
// decoder's sink can write without `await`.  A browser whose handle
// returns a promise is refused, not written to.
export function part_write(id, bytes, at) {
  const n = parts.get(id).write(bytes, { at });
  if (n !== bytes.byteLength) throw new Error(`wrote ${n} of ${bytes.byteLength} bytes`);
}
// The reads of whole blocks in the second pass share one array, so they
// make no garbage.  The decode worker copies each read into WebAssembly
// memory before the next.  A shorter read, at the end of a file, gets a
// new array.  Each read gets a whole array, never a view, as each write
// does (MEM-5).
let read_block = new Uint8Array(0);
export function part_read(id, at, len) {
  if (read_block.length < len) read_block = new Uint8Array(len);
  const b = read_block.length === len ? read_block : new Uint8Array(len);
  const n = parts.get(id).read(b, { at });
  if (n !== len) throw new Error(`read ${n} of ${len} bytes`);
  return b;
}
export async function part_close(id) {
  const h = parts.get(id);
  parts.delete(id);
  await h.flush();
  await h.close();
}
export async function remove_at(path) {
  try {
    const [d, n] = split(path);
    await (await folder(d, false)).removeEntry(n);
  } catch (_) {}
}
