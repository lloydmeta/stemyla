// Drives the stem worker directly from a blank page (`xtask e2e storage`
// and `worker-check`), and looks at what ends up in browser storage.
window.e2e = {
  // Name -> size of every file at the root of the private file system.
  async files() {
    const root = await navigator.storage.getDirectory();
    const out = {};
    for await (const [name, h] of root.entries()) if (h.kind === "file") out[name] = (await h.getFile()).size;
    return out;
  },
  async wipe() {
    const root = await navigator.storage.getDirectory();
    for await (const name of root.keys()) await root.removeEntry(name, { recursive: true });
  },
  async plant(name, bytes) {
    const root = await navigator.storage.getDirectory();
    const w = await (await root.getFileHandle(name, { create: true })).createWritable();
    await w.write(new Uint8Array(bytes));
    await w.close();
  },
  // Writes `bytes` (a Uint8Array) as the file at `path` (such as
  // "sessions/1/mix.f32"), with its folders.
  async plant_at(path, bytes) {
    const parts = path.split("/").filter((s) => s);
    const name = parts.pop();
    const d = await e2e.folder(parts.join("/"));
    const w = await (await d.getFileHandle(name, { create: true })).createWritable();
    await w.write(bytes);
    await w.close();
  },
  // Makes the folder at `path` (such as "sessions/1"), with its parents.
  async folder(path) {
    let d = await navigator.storage.getDirectory();
    for (const p of path.split("/").filter((s) => s)) d = await d.getDirectoryHandle(p, { create: true });
    return d;
  },
  // The bytes of the file at `path`, or null if there is none.
  async read(path) {
    const parts = path.split("/").filter((s) => s);
    const name = parts.pop();
    try {
      let d = await navigator.storage.getDirectory();
      for (const p of parts) d = await d.getDirectoryHandle(p);
      return new Uint8Array(await (await (await d.getFileHandle(name)).getFile()).arrayBuffer());
    } catch (e) {
      if (e.name === "NotFoundError") return null;
      throw e;
    }
  },
  // The bytes of a typed array as base64, so they reach the runner as
  // they are, with no rounding.
  base64(a) {
    const b = new Uint8Array(a.buffer, a.byteOffset, a.byteLength);
    let s = "";
    for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode.apply(null, b.subarray(i, i + 0x8000));
    return btoa(s);
  },
  // A started worker, once it says it is alive.
  start() {
    return new Promise((resolve, reject) => {
      const w = new Worker("/stem-worker_loader.js", { type: "module" });
      w.onerror = (e) => reject(new Error(`worker error: ${e.message}`));
      w.onmessage = (e) => { if (e.data.type === "alive") resolve(w); };
    });
  },
  // Starts a fresh worker and sends `msgs`; resolves once there is one
  // reply ("model", "error", "selftest", "unstored", or "done" after the
  // stems) per message, with every reply, status and log line in order
  // (`at`: ms since `t0`, the call's performance.now()), and the files
  // afterwards.  `opts.progress`, `opts.segment` and `opts.stored` get
  // each reply of their type.  `opts.keep` keeps the worker running
  // (returned as `worker`).
  async ask(msgs, opts = {}) {
    const t0 = performance.now();
    const w = await e2e.start();
    const out = { t0, replies: [], statuses: [], logs: [] };
    const done = new Promise((resolve) => {
      w.onmessage = (e) => {
        const d = e.data;
        const at = Math.round(performance.now() - t0);
        if (d.type === "status") out.statuses.push({ at, text: d.text });
        if (d.type === "log") out.logs.push(d.text);
        if (d.type === "progress" && opts.progress) opts.progress(d);
        if (d.type === "segment" && opts.segment) opts.segment(d);
        if (d.type === "stored" && opts.stored) opts.stored(d);
        if (["model", "error", "selftest", "unstored", "done"].includes(d.type)) {
          const { type, of, ok, loaded, stored, text, secs } = d;
          out.replies.push({ at, type, of, ok, loaded, stored, text, secs });
          if (out.replies.length === msgs.length) resolve();
        }
      };
      w.onerror = (e) => { out.replies.push({ type: "worker-error", text: String(e.message) }); resolve(); };
    });
    for (const m of msgs) w.postMessage(m);
    await done;
    if (opts.keep) out.worker = w; else w.terminate();
    out.files = await e2e.files();
    return out;
  },
};
