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
  // The page's requestAnimationFrame callbacks (eframe asks for one each
  // browser frame, and paints only on request) and the OPFS writes that it
  // made, so a run can see that a save goes on, and how many animation
  // frames it takes.
  // While `__e2e_throttle` is on, each animation frame waits 250 ms first,
  // so the page gets about 4 a second, on any runner.  A save that waits
  // for the UI frame then takes over 400 s for 10 minutes of audio.
  // While `__e2e_long_frame_ms` is above 0, each animation frame holds the
  // main thread that long before the page's callback, as a paint in
  // software does on CI (about 125 ms in CI run 40838459678).
  window.__e2e_frames = 0;
  window.__e2e_throttle = false;
  window.__e2e_long_frame_ms = 0;
  const real_raf = window.requestAnimationFrame.bind(window);
  window.requestAnimationFrame = (f) => {
    const frame = (t) => {
      window.__e2e_frames++;
      const start = performance.now();
      while (performance.now() - start < window.__e2e_long_frame_ms) {}
      f(t);
    };
    if (!window.__e2e_throttle) return real_raf(frame);
    setTimeout(() => real_raf(frame), 250);
    return 0;
  };
  window.__e2e_writes = 0;
  if (window.FileSystemWritableFileStream) {
    const real_write = FileSystemWritableFileStream.prototype.write;
    FileSystemWritableFileStream.prototype.write = function (data) {
      window.__e2e_writes++;
      // A heap reading that starts at each write of the page while a save
      // is measured, so a save of less than a second gets some.  It waits
      // for wasm-bindgen's finalizers as every reading does, so it counts
      // no garbage; the save goes on meanwhile.
      if (window.__e2e_heap_on) window.__e2e_heap().then(window.__e2e_keep);
      return real_write.call(this, data);
    };
  }
  // A number that grows while a save goes on: the page's OPFS writes, and
  // the bytes of the files in saves/, which the worker writes.
  window.__e2e_progress = async () => {
    let bytes = 0;
    try {
      const d = await (await navigator.storage.getDirectory()).getDirectoryHandle("saves");
      for await (const h of d.values()) if (h.kind === "file") bytes += (await h.getFile()).size;
    } catch (_) {}
    return window.__e2e_writes + bytes;
  };
  // The main thread's long tasks (over 50 ms in Chrome), each its start
  // and its length in milliseconds.
  window.__e2e_long = [];
  try {
    new PerformanceObserver((list) => {
      for (const e of list.getEntries()) window.__e2e_long.push([e.startTime, e.duration]);
    }).observe({ entryTypes: ["longtask"] });
  } catch (_) {}
  const long_tasks_seen = () => typeof PerformanceObserver === "function"
    && (PerformanceObserver.supportedEntryTypes || []).includes("longtask");
  // The lengths of the long tasks that started between the mark of the
  // last save and its offer, or null where the browser has no long tasks.
  window.__e2e_long_in_save = () => !long_tasks_seen() ? null : window.__e2e_long
    .filter(([t]) => t >= (window.__e2e_t_at_start || 0) && t <= (window.__e2e_t_offer || Infinity))
    .map(([, d]) => d);
  const tick_ms = (ms) => new Promise((r) => setTimeout(r, ms));
  // A calibration of the long tasks: the length of the longest one seen
  // after the page holds its main thread for 120 ms, or null.
  window.__e2e_long_calibrate = async () => {
    if (!long_tasks_seen()) return null;
    // A task of the page's own: a script that WebDriver runs is not seen.
    const t = performance.now();
    setTimeout(() => {
      const start = performance.now();
      while (performance.now() - start < 120) {}
    }, 0);
    // The observer gets its entries some time after the task.
    await tick_ms(1000);
    const seen = window.__e2e_long.filter(([s]) => s >= t).map(([, d]) => d);
    return seen.length === 0 ? 0 : Math.max(...seen);
  };
  HTMLAnchorElement.prototype.click = function () {
    window.__e2e_t_offer = performance.now();
    window.__e2e_throttle = false;
    const blob = blobs.get(this.href) || null;
    blobs.delete(this.href);
    const save = { name: this.download, size: blob ? blob.size : -1, mime: blob === null ? "" : blob.type,
      frames: window.__e2e_frames - (window.__e2e_frames_at_start || 0),
      ms: Math.round(performance.now() - (window.__e2e_t_at_start || 0)) };
    window.__e2e_saves.push(save);
    // Only the last file is kept, for the checks; the page's own object
    // URL keeps it for 60 s anyway.
    window.__e2e_last = blob;
    console.log(`e2e: offered ${JSON.stringify(save)}`);
  };
  const tick = (ms) => new Promise((r) => setTimeout(r, ms));
  // The JS heap in bytes once garbage is collected: Chrome with
  // --enable-precise-memory-info and --js-flags=--expose-gc.  A collection
  // queues the finalizers of wasm-bindgen's JS objects, which run in a
  // later task.  So it collects three times with a task of 20 ms after
  // each, then once more, and reads.
  window.__e2e_heap = async () => {
    if (typeof gc !== "function" || !performance.memory) return null;
    for (let i = 0; i < 3; i++) {
      gc();
      await tick(20);
    }
    gc();
    return performance.memory.usedJSHeapSize;
  };
  // Keeps the largest reading of the save.
  window.__e2e_keep = (v) => {
    if (v !== null && (window.__e2e_heap_max === null || v > window.__e2e_heap_max)) window.__e2e_heap_max = v;
  };
  // The start of a save, just before the click of its menu item: the
  // offer gives the animation frames (requestAnimationFrame callbacks) and
  // the milliseconds since.  With `throttle`, the frames are throttled
  // until the offer.
  window.__e2e_mark = (throttle) => {
    window.__e2e_throttle = !!throttle;
    window.__e2e_frames_at_start = window.__e2e_frames;
    window.__e2e_t_at_start = performance.now();
    window.__e2e_t_offer = null;
  };
  // Reads the heap until `__e2e_stop_heap`, one reading after another,
  // and keeps the largest value.
  window.__e2e_start_heap = () => {
    window.__e2e_heap_max = null;
    window.__e2e_heap_on = true;
    (async () => {
      while (window.__e2e_heap_on) {
        window.__e2e_keep(await window.__e2e_heap());
        await tick(250);
      }
    })();
  };
  window.__e2e_stop_heap = async () => {
    window.__e2e_heap_on = false;
    window.__e2e_keep(await window.__e2e_heap());
    return window.__e2e_heap_max;
  };
})();
