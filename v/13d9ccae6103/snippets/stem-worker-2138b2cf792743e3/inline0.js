
async function session_folder(path) {
  let d = await navigator.storage.getDirectory();
  for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p);
  return d;
}
// `write` returns the bytes written.  Old browsers return a promise, so
// it is awaited, as `truncate`, `flush` and `close` are.
async function put(h, bytes, at) {
  const n = await h.write(bytes, { at });
  if (n !== bytes.byteLength) throw new Error(`wrote ${n} of ${bytes.byteLength} bytes`);
}
// The session folder must exist: it is never made here, so a forgotten
// session does not come back.
// `ours`: the files that this writer made or holds.  A file that another
// writer holds is not ours, so a failed open never removes it.
export async function stems_open(session, files, header) {
  const dir = await (await session_folder(session)).getDirectoryHandle("stems", { create: true });
  const w = { dir, handles: [], ours: [] };
  try {
    for (const name of files) {
      let fh;
      try { fh = await dir.getFileHandle(name); }
      catch (e) {
        if (e.name !== "NotFoundError") throw e;
        fh = await dir.getFileHandle(name, { create: true });
        w.ours.push(name);
      }
      const h = await fh.createSyncAccessHandle();
      w.handles.push(h);
      if (!w.ours.includes(name)) w.ours.push(name);
      await h.truncate(0);
      await put(h, header, 0);
    }
  } catch (e) {
    await stems_abandon(w);
    throw e;
  }
  return w;
}
export async function stems_write(w, i, bytes, at) {
  await put(w.handles[i], bytes, at);
}
export async function stems_finish(w, header) {
  for (const h of w.handles) {
    await put(h, header, 0);
    await h.flush();
  }
  for (const h of w.handles) await h.close();
}
// Closes each handle and removes each file that this writer made or held.
export async function stems_abandon(w) {
  for (const h of w.handles) {
    try { await h.close(); } catch (_) {}
  }
  for (const name of w.ours) {
    try { await w.dir.removeEntry(name); } catch (_) {}
  }
}
