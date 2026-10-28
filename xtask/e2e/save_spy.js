// Spies on the page's downloads for `xtask e2e save` (M8 of
// plans/active/stream-from-storage.md): each link click that a save makes
// is kept, not downloaded.  Served by `xtask serve --inject`, so it runs
// before the app, in every build.
(() => {
  const blobs = new Map();
  const real_url = URL.createObjectURL;
  URL.createObjectURL = (b) => {
    const u = real_url(b);
    if (b instanceof Blob) blobs.set(u, b);
    return u;
  };
  window.__e2e_saves = [];
  HTMLAnchorElement.prototype.click = function () {
    const blob = blobs.get(this.href) || null;
    blobs.delete(this.href);
    const save = { name: this.download, size: blob ? blob.size : -1, mime: blob === null ? "" : blob.type };
    window.__e2e_saves.push(save);
    // Only the last file is kept, for the checks; the page's own object
    // URL keeps it for 60 s anyway.
    window.__e2e_last = blob;
    console.log(`e2e: offered ${JSON.stringify(save)}`);
  };
  const tick = (ms) => new Promise((r) => setTimeout(r, ms));
  // The JS heap in bytes once garbage is collected: Chrome with
  // --enable-precise-memory-info and --js-flags=--expose-gc.  A first
  // collection queues the finalizers of wasm-bindgen's JS objects, which
  // run in a later task, so it collects again after them.
  window.__e2e_heap = async () => {
    if (typeof gc !== "function" || !performance.memory) return null;
    for (let i = 0; i < 3; i++) {
      gc();
      await tick(20);
    }
    gc();
    return performance.memory.usedJSHeapSize;
  };
  // Reads the heap until `__e2e_stop_heap`, one reading after another,
  // and keeps the largest value.
  window.__e2e_start_heap = () => {
    window.__e2e_heap_max = null;
    window.__e2e_heap_on = true;
    (async () => {
      while (window.__e2e_heap_on) {
        const v = await window.__e2e_heap();
        if (v !== null && (window.__e2e_heap_max === null || v > window.__e2e_heap_max)) window.__e2e_heap_max = v;
        await tick(250);
      }
    })();
  };
  window.__e2e_stop_heap = async () => {
    window.__e2e_heap_on = false;
    const v = await window.__e2e_heap();
    if (v !== null && (window.__e2e_heap_max === null || v > window.__e2e_heap_max)) window.__e2e_heap_max = v;
    return window.__e2e_heap_max;
  };
})();
