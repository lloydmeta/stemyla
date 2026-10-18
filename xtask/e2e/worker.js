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
  // A started worker, once it says it is alive.
  start() {
    return new Promise((resolve, reject) => {
      const w = new Worker("/stem-worker_loader.js", { type: "module" });
      w.onerror = (e) => reject(new Error(`worker error: ${e.message}`));
      w.onmessage = (e) => { if (e.data.type === "alive") resolve(w); };
    });
  },
  // Starts a fresh worker and sends `msgs`; resolves once there is one
  // reply ("model", "error", "selftest", or "done" after the stems) per message, with every reply,
  // status and log line in order (`at`: ms since `t0`, the call's
  // performance.now()), and the files
  // afterwards.  `opts.keep` keeps the worker running (returned as `worker`).
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
        if (d.type === "stem" && opts.stem) opts.stem(d);
        if (["model", "error", "selftest", "done"].includes(d.type)) {
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
