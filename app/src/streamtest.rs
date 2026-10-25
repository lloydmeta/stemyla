//! `?streamtest`: a spike that measures playback from browser storage
//! (OPFS) on this device, for M1 of `plans/active/stream-from-storage.md`
//! (decisions 0016 and 0017).  Web only.
//!
//! It writes the files of a separated song to a scratch folder: six 16-bit
//! stems, a 32-bit float original lane and a 16-bit classic-drums stem.
//! Then it plays them as the app's output would: two buffers of 2,048
//! frames queued, each filled from a read-ahead buffer that async reads on
//! the main thread keep full.  It logs one line per measure, with the
//! prefix `streamtest:`, and removes the scratch folder.
//!
//! Page parameters: `streamtest` (on), `st_minutes` (length of the files,
//! default 30), `st_play` (seconds of each playback run, default 300) and
//! `st_seeks` (default 50).  The run starts from a button, since iOS starts
//! audio only after a tap.

use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
const RATE = 44100;
const FRAMES_PER_BUFFER = 2048;
const SCRATCH = "streamtest";
// Name and bytes per stereo frame: 16-bit stems take 4, the f32 original 8.
const FILES = [["bass", 4], ["drums", 4], ["other", 4], ["vocals", 4], ["guitar", 4], ["piano", 4],
  ["original", 8], ["classic drums", 4]];

export function st_cfg(minutes, play_s, seeks, spy) {
  return { minutes, play_s, seeks, buffer_s: 10, block_ms: 50, block_every_s: 3, spy };
}

export function st_url_cfg() {
  const q = new URLSearchParams(location.search);
  if (!q.has("streamtest")) return null;
  const num = (k, d) => { const v = parseFloat(q.get(k)); return v > 0 ? v : d; };
  return st_cfg(num("st_minutes", 30), num("st_play", 300), Math.round(num("st_seeks", 50)), false);
}

// Nearest-rank percentile of `v` (milliseconds), to 0.1 ms.
function pct(v, p) {
  if (!v.length) return "-";
  const s = [...v].sort((a, b) => a - b);
  return s[Math.max(0, Math.ceil(p * s.length) - 1)].toFixed(1);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function remove_scratch() {
  try { await (await navigator.storage.getDirectory()).removeEntry(SCRATCH, { recursive: true }); }
  catch (e) { if (e.name !== "NotFoundError") throw e; }
}

// Writes each file in blocks of 8 MB.  Every write gets a whole array
// (MEM-5): the full block, or a copy of its start for the last one.
async function write_files(dir, frames) {
  const block = new Uint8Array(8 << 20);
  for (let i = 0; i < block.length; i++) block[i] = (i * 31) & 255;
  let bytes = 0;
  const t0 = performance.now();
  for (const [name, bpf] of FILES) {
    const w = await (await dir.getFileHandle(name, { create: true })).createWritable();
    try {
      for (let left = frames * bpf; left > 0;) {
        const n = Math.min(left, block.length);
        await w.write(n === block.length ? block : block.slice(0, n));
        left -= n;
        bytes += n;
      }
      await w.close();
    } catch (e) {
      try { await w.abort(); } catch (_) {}
      throw e;
    }
  }
  return [bytes, (performance.now() - t0) / 1000];
}

// The read-ahead buffer of decision 0017: frames [start, end) of every
// file, in rings of `cap` frames, filled one second at a time by one async
// read of all the files.  `pos` is the play head.
class Ahead {
  constructor(files, frames, buffer_s) {
    this.files = files;
    this.frames = frames;
    this.cap = Math.round(buffer_s * RATE);
    this.rings = files.map((f) => new Uint8Array(this.cap * f.bpf));
    this.start = this.end = this.pos = 0;
    this.gen = 0;
    this.busy = false;
    this.times = [];
    this.failed = null;
    this.waiters = [];
  }
  seek(pos) {
    this.gen++;
    this.start = this.end = this.pos = pos;
    this.pump();
  }
  // Resolves when `cond()` holds after a read, or at once if it holds now.
  until(cond) {
    return cond() ? Promise.resolve() : new Promise((r) => this.waiters.push([cond, r]));
  }
  pump() {
    if (this.busy || this.failed) return;
    this.start = Math.max(this.start, Math.min(this.pos, this.end));
    if (this.end >= this.frames || this.end + RATE - this.start > this.cap) return;
    this.busy = true;
    const gen = this.gen, from = this.end, to = Math.min(from + RATE, this.frames);
    const t0 = performance.now();
    Promise.all(this.files.map((f) => f.file.slice(from * f.bpf, to * f.bpf).arrayBuffer())).then((bufs) => {
      this.times.push(performance.now() - t0);
      if (gen !== this.gen) return;
      bufs.forEach((b, i) => {
        const bpf = this.files[i].bpf, ring = this.rings[i], src = new Uint8Array(b);
        const at = (from % this.cap) * bpf, first = Math.min(src.length, ring.length - at);
        ring.set(src.subarray(0, first), at);
        if (first < src.length) ring.set(src.subarray(first), 0);
      });
      this.end = to;
    }, (e) => { this.failed = (e && e.name) || String(e); }).finally(() => {
      this.busy = false;
      this.waiters = this.waiters.filter(([cond, r]) => (cond() || this.failed ? (r(), false) : true));
      this.pump();
    });
  }
}

// Plays `seconds` from `from` at `tempo`, as cpal's Web Audio host does:
// two buffers queued, the next scheduled when one ends.  A buffer whose
// start time has passed is late (`live.rs` leaves it silent).  A buffer
// whose frames the read-ahead buffer does not hold is an underrun.  With
// `block_ms`, the main thread is busy that long every `block_every_s`.
async function play(ahead, ctx, from, seconds, tempo, block_ms, block_every_s) {
  ahead.seek(from);
  await ahead.until(() => ahead.end - ahead.start + RATE > ahead.cap || ahead.end >= ahead.frames);
  let under = 0, late = 0, buffers = 0, pos = from;
  const rate = ctx ? ctx.sampleRate : RATE;
  const step = Math.round((FRAMES_PER_BUFFER * RATE * tempo) / rate);
  const period = FRAMES_PER_BUFFER / rate;
  const blocker = block_ms > 0 ? setInterval(() => {
    const until = performance.now() + block_ms;
    while (performance.now() < until) {}
  }, block_every_s * 1000) : null;
  // Takes one buffer's frames from the read-ahead buffer; false at the end.
  const take = () => {
    if (pos + step > ahead.frames) return false;
    if (pos + step > ahead.end || pos < ahead.start) under++;
    pos += step;
    ahead.pos = pos;
    ahead.pump();
    buffers++;
    return buffers * period < seconds;
  };
  await new Promise((done) => {
    if (ctx) {
      const silence = ctx.createBuffer(2, FRAMES_PER_BUFFER, rate);
      let t = ctx.currentTime + 0.05, live = 2;
      const schedule = () => {
        if (!take()) { if (--live === 0) done(); return; }
        if (t < ctx.currentTime) { late++; t = ctx.currentTime; }
        const src = ctx.createBufferSource();
        src.buffer = silence;
        src.connect(ctx.destination);
        src.onended = schedule;
        src.start(t);
        t += period;
      };
      schedule();
      schedule();
    } else {
      // No running audio context: a timer stands in for the 'ended' events.
      let last = performance.now();
      const tick = () => {
        const now = performance.now();
        if (now - last > 2 * period * 1000) late++;
        last = now;
        if (take()) setTimeout(tick, period * 1000); else done();
      };
      tick();
    }
  });
  if (blocker !== null) clearInterval(blocker);
  return { under, late, played: buffers * period };
}

// A worker appends 1 MB every 10 ms to `name` through a sync access
// handle while the main thread reads the file's end.  With `spy`, the
// worker records what each `FileSystemSyncAccessHandle.write` gets.
const WORKER = `
onmessage = async (e) => {
  const { name, chunks, spy } = e.data;
  const record = [];
  try {
    if (spy) {
      const p = FileSystemSyncAccessHandle.prototype, real = p.write;
      p.write = function (d, o) {
        record.push(ArrayBuffer.isView(d) && d.byteOffset === 0 && d.byteLength === d.buffer.byteLength ? "whole" : "view");
        return real.call(this, d, o);
      };
    }
    const dir = await (await navigator.storage.getDirectory()).getDirectoryHandle("${SCRATCH}");
    const h = await (await dir.getFileHandle(name, { create: true })).createSyncAccessHandle();
    postMessage({ open: true });
    let at = 0;
    for (let i = 0; i < chunks; i++) {
      at += h.write(new Uint8Array(1 << 20).fill(i & 255), { at });
      h.flush();
      await new Promise((r) => setTimeout(r, 10));
    }
    h.close();
    postMessage({ done: true, bytes: at, record: record.join(" ") });
  } catch (err) {
    postMessage({ error: (err && err.name) || String(err) });
  }
};`;

async function concurrent(dir, spy) {
  const url = URL.createObjectURL(new Blob([WORKER], { type: "text/javascript" }));
  const worker = new Worker(url);
  const state = { open: false, done: false };
  worker.onmessage = (e) => Object.assign(state, e.data);
  worker.postMessage({ name: "concurrent", chunks: 50, spy });
  const deadline = performance.now() + 15000;
  while (!state.open && !state.error && performance.now() < deadline) await sleep(5);
  let ok = 0;
  const errors = new Set(), sizes = [];
  while (state.open && !state.done && !state.error && performance.now() < deadline) {
    try {
      const f = await (await dir.getFileHandle("concurrent")).getFile();
      sizes.push(f.size);
      await f.slice(Math.max(0, f.size - 4096), f.size).arrayBuffer();
      ok++;
    } catch (e) {
      errors.add((e && e.name) || String(e));
    }
    await sleep(20);
  }
  worker.terminate();
  URL.revokeObjectURL(url);
  const grew = sizes.length > 1 && sizes[sizes.length - 1] > sizes[0];
  return { ok, errors: [...errors].join(",") || "none", grew, bytes: state.bytes || 0,
    worker: state.error || (state.done ? "done" : "timed out"), record: state.record || "" };
}

export async function st_run(cfg, line) {
  await remove_scratch();
  const dir = await (await navigator.storage.getDirectory()).getDirectoryHandle(SCRATCH, { create: true });
  try {
    const frames = Math.round(cfg.minutes * 60 * RATE);
    const [bytes, secs] = await write_files(dir, frames);
    line(`write: ${FILES.length} files, ${(bytes / 1e6).toFixed(0)} MB in ${secs.toFixed(1)} s (${(bytes / 1e6 / secs).toFixed(0)} MB/s)`);
    const files = await Promise.all(FILES.map(async ([name, bpf]) => ({ bpf, file: await (await dir.getFileHandle(name)).getFile() })));
    let ctx = null;
    try {
      ctx = new AudioContext();
      await Promise.race([ctx.resume(), sleep(1000)]);
      if (ctx.state !== "running") { await ctx.close(); ctx = null; }
    } catch (_) { ctx = null; }
    const clock = ctx ? `audio at ${ctx.sampleRate} Hz` : "timer";
    for (const tempo of [1, 1.5]) {
      for (const block of [0, cfg.block_ms]) {
        const ahead = new Ahead(files, frames, cfg.buffer_s);
        const r = await play(ahead, ctx, 0, cfg.play_s, tempo, block, cfg.block_every_s);
        const with_block = block > 0 ? `, ${block} ms block every ${cfg.block_every_s} s` : "";
        line(`playback: tempo ${tempo}${with_block}: underruns ${r.under}, late buffers ${r.late}, ` +
          `${r.played.toFixed(1)} s played, clock ${clock}` + (ahead.failed ? `, read error ${ahead.failed}` : ""));
        line(`reads: tempo ${tempo}${with_block}: one second of ${FILES.length} files: ` +
          `p50 ${pct(ahead.times, 0.5)} ms, p99 ${pct(ahead.times, 0.99)} ms, max ${pct(ahead.times, 1)} ms, ${ahead.times.length} reads`);
      }
    }
    if (ctx) await ctx.close();
    const ahead = new Ahead(files, frames, cfg.buffer_s);
    const seeks = [];
    for (let i = 0; i < cfg.seeks; i++) {
      // Spread over the song, so each target is outside the buffer.
      const to = Math.floor(((i * 7919) % cfg.seeks) / cfg.seeks * Math.max(1, frames - RATE));
      const t0 = performance.now();
      ahead.seek(to);
      await ahead.until(() => ahead.end > to);
      seeks.push(performance.now() - t0);
    }
    line(`seeks: to the first frame of the new position: p50 ${pct(seeks, 0.5)} ms, p99 ${pct(seeks, 0.99)} ms, ${seeks.length} seeks` +
      (ahead.failed ? `, read error ${ahead.failed}` : ""));
    const c = await concurrent(dir, cfg.spy);
    line(`concurrent: reads while a worker writes: ${c.ok} ok, errors ${c.errors}, size grew ${c.grew}, ` +
      `worker ${c.worker}, ${(c.bytes / 1e6).toFixed(0)} MB written` + (cfg.spy ? `, worker writes [${c.record}]` : ""));
  } finally {
    await remove_scratch();
  }
  line("done");
}
"#)]
extern "C" {
    fn st_cfg(minutes: f64, play_s: f64, seeks: u32, spy: bool) -> JsValue;
    fn st_url_cfg() -> JsValue;
    #[wasm_bindgen(catch)]
    async fn st_run(cfg: &JsValue, line: &Closure<dyn FnMut(String)>) -> Result<JsValue, JsValue>;
}

/// What a run measures, and how long it takes.
pub struct Settings(JsValue);

impl Settings {
    /// The settings in the page address, if it asks for the test.
    pub fn from_url() -> Option<Self> {
        let cfg = st_url_cfg();
        (!cfg.is_null()).then_some(Self(cfg))
    }

    /// Files of `minutes`, `play_s` seconds of each playback run and
    /// `seeks` seeks.  With `spy`, the worker reports what each of its
    /// writes got (MEM-5).
    pub fn new(minutes: f64, play_s: f64, seeks: u32, spy: bool) -> Self {
        Self(st_cfg(minutes, play_s, seeks, spy))
    }
}

/// Runs the test and gives each result line, without the prefix, to
/// `line`.  The last line is "done".
pub async fn run(settings: &Settings, line: impl FnMut(String) + 'static) -> Result<(), String> {
    let line: Closure<dyn FnMut(String)> = Closure::new(line);
    st_run(&settings.0, &line)
        .await
        .map(|_| ())
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
}

/// The page's state of the test: whether it runs, and its last line.
#[derive(Default)]
pub struct Status {
    pub running: bool,
    pub last: Option<String>,
}

/// Starts a run that logs each line as `streamtest: ...` and keeps the
/// last one in `status`.
pub fn start(settings: Settings, status: Rc<RefCell<Status>>, ctx: eframe::egui::Context) {
    status.borrow_mut().running = true;
    wasm_bindgen_futures::spawn_local(async move {
        let shown = status.clone();
        let repaint = ctx.clone();
        let result = run(&settings, move |text| {
            log::info!("streamtest: {text}");
            shown.borrow_mut().last = Some(text);
            repaint.request_repaint();
        })
        .await;
        let mut s = status.borrow_mut();
        if let Err(e) = result {
            log::error!("streamtest: failed: {e}");
            s.last = Some(format!("failed: {e}"));
        }
        s.running = false;
        ctx.request_repaint();
    });
}

/// Browser tests (`cargo run -p xtask -- web-test`).
#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen(inline_js = r#"
let real = null, record = null;
// Records what each FileSystemWritableFileStream.write gets: "whole",
// "view", "blob" or a type name.
export function st_spy_on() {
  const p = FileSystemWritableFileStream.prototype;
  real = p.write;
  record = [];
  p.write = function (d) {
    if (d instanceof Blob) record.push("blob");
    else if (ArrayBuffer.isView(d)) record.push(d.byteOffset === 0 && d.byteLength === d.buffer.byteLength ? "whole" : "view");
    else record.push(typeof d);
    return real.call(this, d);
  };
}
export function st_spy_off() {
  FileSystemWritableFileStream.prototype.write = real;
  real = null;
  return record.join(" ");
}
export async function st_scratch_exists() {
  try { await (await navigator.storage.getDirectory()).getDirectoryHandle("streamtest"); return true; }
  catch (e) { return false; }
}
"#)]
    extern "C" {
        fn st_spy_on();
        fn st_spy_off() -> String;
        async fn st_scratch_exists() -> JsValue;
    }

    /// The spike with a short song: each measure logs its line, every
    /// write gets a whole array (MEM-5), and the scratch folder is gone.
    #[wasm_bindgen_test]
    async fn the_spike_logs_each_measure_and_leaves_no_files() {
        let lines = Rc::new(RefCell::new(Vec::new()));
        let seen = lines.clone();
        st_spy_on();
        // 0.2 minutes: 13 MB of files, so the last write of each file is
        // shorter than a block.
        let result = run(&Settings::new(0.2, 1.0, 3, true), move |l| {
            seen.borrow_mut().push(l)
        })
        .await;
        let writes = st_spy_off();
        result.expect("the spike runs");
        let lines = lines.take();
        let count = |prefix: &str| lines.iter().filter(|l| l.starts_with(prefix)).count();
        assert_eq!(count("write: 8 files, "), 1, "{lines:#?}");
        assert_eq!(count("playback: tempo "), 4, "{lines:#?}");
        assert_eq!(count("reads: tempo "), 4, "{lines:#?}");
        assert_eq!(count("seeks: "), 1, "{lines:#?}");
        assert_eq!(count("concurrent: "), 1, "{lines:#?}");
        assert_eq!(lines.last().map(String::as_str), Some("done"));
        for l in lines.iter().filter(|l| l.starts_with("playback: ")) {
            assert!(!l.contains("read error"), "{l}");
            assert!(!l.contains(" 0.0 s played"), "{l}");
        }
        let seeks = lines
            .iter()
            .find(|l| l.starts_with("seeks: "))
            .expect("a seek line");
        assert!(seeks.ends_with(", 3 seeks"), "{seeks}");
        let reads = lines
            .iter()
            .find(|l| l.starts_with("reads: tempo 1:"))
            .expect("a read line");
        assert!(!reads.contains("p50 -"), "{reads}");
        let concurrent = lines
            .iter()
            .find(|l| l.starts_with("concurrent: "))
            .expect("a concurrent line");
        assert!(concurrent.contains("worker done"), "{concurrent}");
        let worker_writes = concurrent
            .split("worker writes [")
            .nth(1)
            .expect("the worker's record");
        assert!(worker_writes.starts_with("whole"), "{concurrent}");
        assert!(!worker_writes.contains("view"), "{concurrent}");
        assert!(
            !writes.is_empty() && writes.split(' ').all(|w| w == "whole"),
            "MEM-5: {writes}"
        );
        assert_eq!(st_scratch_exists().await.as_bool(), Some(false));
    }
}
