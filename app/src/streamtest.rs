//! `?streamtest`: a spike that measures playback from browser storage
//! (OPFS) on this device, for M1 of `plans/completed/stream-from-storage.md`
//! (decisions 0016 and 0017).  Web only.
//!
//! It writes the files of a separated song to a scratch folder: six 16-bit
//! stems, a 32-bit float original lane and a 16-bit classic-drums stem.
//! Then it plays them as cpal's Web Audio host plays: two buffers of
//! `engine::slot::WEB_BUFFER_FRAMES` frames queued, on cpal's schedule.  Async reads on the main thread fill
//! a read-ahead buffer, and each buffer takes its frames from it.  The test
//! logs one line per measure, with the prefix `streamtest:`, and removes
//! the scratch folder.
//!
//! Page parameters: `streamtest` (on), `st_minutes` (length of the files,
//! default 30), `st_play` (seconds of each playback run, default 300) and
//! `st_seeks` (the number of seeks, default 50).  The run starts from a
//! button.  iOS starts audio only in a tap's handler, so the button makes
//! the audio context there.

use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
const RATE = 44100;
const SCRATCH = "streamtest";
// Name and bytes per stereo frame: 16-bit stems take 4, the f32 original 8.
const FILES = [["bass", 4], ["drums", 4], ["other", 4], ["vocals", 4], ["guitar", 4], ["piano", 4],
  ["original", 8], ["classic drums", 4]];

// `block_ms` is shorter than one buffer (about 64 to 70 ms), the slack
// that cpal has at each 'ended' event.  `frames_per_buffer` is the page's
// output buffer (`WEB_BUFFER_FRAMES`).  `read_delay_ms` and
// `fail_read_at` (the read of each read-ahead buffer that fails, from 1)
// are for tests.
export function st_cfg(minutes, play_s, seeks, max_tempo, frames_per_buffer) {
  return { minutes, play_s, seeks, max_tempo, frames_per_buffer, buffer_s: 10, block_ms: 30, block_every_s: 3, block_bytes: 8 << 20,
    read_delay_ms: 0, fail_read_at: 0, spy: false };
}

export function st_cfg_set(cfg, key, value) { cfg[key] = value; }

// The settings in a page address's query, or null without `streamtest`.
export function st_query_cfg(search, max_tempo, frames_per_buffer) {
  const q = new URLSearchParams(search);
  if (!q.has("streamtest")) return null;
  const num = (k, d) => { const v = parseFloat(q.get(k)); return v > 0 ? v : d; };
  return st_cfg(num("st_minutes", 30), num("st_play", 300), Math.round(num("st_seeks", 50)), max_tempo,
    frames_per_buffer);
}

// Made in a tap's handler: Safari starts audio only there.
export function st_audio() {
  try {
    const ctx = new AudioContext();
    ctx.resume().catch(() => {});
    return ctx;
  } catch (_) {
    return null;
  }
}

// Nearest-rank percentile of `v` (milliseconds), to 0.1 ms.
function pct(v, p) {
  if (!v.length) return "-";
  const s = [...v].sort((a, b) => a - b);
  return s[Math.max(0, Math.ceil(p * s.length) - 1)].toFixed(1);
}

// The times of a line: p50, p99 and the maximum.
export function st_times_text(v) {
  return `p50 ${pct(v, 0.5)} ms, p99 ${pct(v, 0.99)} ms, max ${pct(v, 1)} ms`;
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function remove_scratch() {
  try { await (await navigator.storage.getDirectory()).removeEntry(SCRATCH, { recursive: true }); }
  catch (e) { if (e.name !== "NotFoundError") throw e; }
}

// Writes each file in blocks of `block_bytes`.  Every write gets a whole
// array (MEM-5): the full block, or a copy of its start for the last one.
async function write_files(dir, frames, block_bytes) {
  const block = new Uint8Array(block_bytes);
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
// read of all the files.  `pos` is the play head.  After a failed read it
// reads no more, and every wait ends.
class Ahead {
  constructor(files, frames, cfg) {
    this.files = files;
    this.frames = frames;
    this.cap = Math.round(cfg.buffer_s * RATE);
    this.delay_ms = cfg.read_delay_ms;
    this.fail_at = cfg.fail_read_at;
    this.rings = files.map((f) => new Uint8Array(this.cap * f.bpf));
    this.start = this.end = this.pos = 0;
    this.gen = 0;
    this.reads = 0;
    this.busy = false;
    this.paused = false;
    this.times = [];
    this.failed = null;
    this.waiters = [];
  }
  seek(pos) {
    this.gen++;
    this.paused = false;
    this.start = this.end = this.pos = pos;
    this.pump();
  }
  // Resolves when `cond()` holds after a read, at once if it holds now,
  // and at once after a failed read.
  until(cond) {
    return cond() || this.failed ? Promise.resolve() : new Promise((r) => this.waiters.push([cond, r]));
  }
  // Waits for the read in flight, and starts no other until a seek.
  idle() {
    this.paused = true;
    return this.until(() => !this.busy);
  }
  pump() {
    if (this.busy || this.failed || this.paused) return;
    this.start = Math.max(this.start, Math.min(this.pos, this.end));
    if (this.end >= this.frames || this.end + RATE - this.start > this.cap) return;
    this.busy = true;
    const gen = this.gen, from = this.end, to = Math.min(from + RATE, this.frames);
    const t0 = performance.now();
    let read = ++this.reads === this.fail_at
      ? Promise.reject(new DOMException("failed by the test", "NotReadableError"))
      : Promise.all(this.files.map((f) => f.file.slice(from * f.bpf, to * f.bpf).arrayBuffer()));
    if (this.delay_ms > 0) read = read.then((b) => sleep(this.delay_ms).then(() => b));
    read.then((bufs) => {
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

// Plays `seconds` from frame `from` at `tempo`, as cpal's Web Audio host
// does.  Two buffers are queued, and each 'ended' event schedules the next
// at the end of the one before.  The first starts at now + base latency +
// one buffer, and the schedule never moves back to the clock.  A buffer
// whose start has passed is late: it starts at once.  The two chains make
// late buffers two at a time (a late pair).  `live.rs` fills one of each
// late pair, and leaves silent the one that would play over it.
// A buffer whose frames the read-ahead buffer does not hold is an
// underrun.  With `block_ms`, the main thread is busy that long every
// `block_every_s`.  Unlike the app's callback, `take` here also starts
// reads, and it copies no frames.
async function play(ahead, ctx, from, seconds, tempo, block_ms, block_every_s, frames_per_buffer) {
  ahead.seek(from);
  await ahead.until(() => ahead.end - ahead.start + RATE > ahead.cap || ahead.end >= ahead.frames);
  let under = 0, late = 0, buffers = 0, blocks = 0, pos = from, interrupted = false;
  const rate = ctx ? ctx.sampleRate : RATE;
  const step = Math.round((frames_per_buffer * RATE * tempo) / rate);
  const period = frames_per_buffer / rate;
  const total = Math.floor(seconds / period);
  const blocker = block_ms > 0 ? setInterval(() => {
    const until = performance.now() + block_ms;
    while (performance.now() < until) {}
    blocks++;
  }, block_every_s * 1000) : null;
  // Takes one buffer's frames from the read-ahead buffer; false when the
  // run or the file has ended.
  const take = () => {
    if (buffers >= total || pos + step > ahead.frames) return false;
    if (pos + step > ahead.end || pos < ahead.start) under++;
    pos += step;
    ahead.pos = pos;
    ahead.pump();
    buffers++;
    return true;
  };
  await new Promise((done) => {
    let live = 2;
    const finish = () => { if (--live === 0) done(); };
    if (ctx) {
      const silence = ctx.createBuffer(2, frames_per_buffer, rate);
      let t = 0;
      // iOS can suspend or interrupt the context, and then no 'ended'
      // event comes.  The run stops and says so.
      const watch = setInterval(() => {
        if (ctx.state === "running") return;
        interrupted = true;
        clearInterval(watch);
        done();
      }, 250);
      const schedule = () => {
        if (interrupted) return;
        if (!take()) {
          if (live === 1) clearInterval(watch);
          return finish();
        }
        if (t === 0) t = ctx.currentTime + (ctx.baseLatency || 0) + period;
        else if (t < ctx.currentTime) late++;
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
      // No running audio context: timers stand in for the 'ended' events,
      // on the same schedule.  Buffer k starts at t0 + k periods.  Its
      // timer is due when buffer k - 2 ends, at t0 + (k - 1) periods.
      const t0 = performance.now() + period * 1000;
      let k = 0;
      const schedule = () => {
        if (!take()) return finish();
        if (performance.now() > t0 + k * period * 1000) late++;
        k++;
        // Buffer k + 1 is next on this chain; buffer k - 1 ends at t0 + k periods.
        setTimeout(schedule, Math.max(0, t0 + k * period * 1000 - performance.now()));
      };
      schedule();
      schedule();
    }
  });
  if (blocker !== null) clearInterval(blocker);
  return { under, late, blocks, interrupted, played: buffers * period, end: pos, period };
}

// A worker appends 1 MB every 10 ms to `name` through a sync access
// handle while the main thread reads the file's last byte.  Chunk i holds
// the byte i & 255.  With `spy`, the worker records what each
// `FileSystemSyncAccessHandle.write` gets.
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
  let right = 0, wrong = 0, failed = 0;
  const errors = new Set(), sizes = [];
  while (state.open && !state.done && !state.error && performance.now() < deadline) {
    try {
      const f = await (await dir.getFileHandle("concurrent")).getFile();
      sizes.push(f.size);
      if (f.size >= 1 << 20) {
        const last = new Uint8Array(await f.slice(f.size - 1, f.size).arrayBuffer())[0];
        if (last === ((Math.ceil(f.size / (1 << 20)) - 1) & 255)) right++; else wrong++;
      }
    } catch (e) {
      failed++;
      errors.add((e && e.name) || String(e));
    }
    await sleep(20);
  }
  worker.terminate();
  URL.revokeObjectURL(url);
  const grew = sizes.length > 1 && sizes[sizes.length - 1] > sizes[0];
  return { right, wrong, failed, errors: [...errors].join(",") || "none", grew, bytes: state.bytes || 0,
    worker: state.error || (state.done ? "done" : "timed out"), record: state.record || "" };
}

export async function st_run(cfg, audio, line) {
  let ctx = audio || null;
  try {
    await remove_scratch();
    const dir = await (await navigator.storage.getDirectory()).getDirectoryHandle(SCRATCH, { create: true });
    const frames = Math.round(cfg.minutes * 60 * RATE);
    const [bytes, secs] = await write_files(dir, frames, cfg.block_bytes);
    line(`write: ${FILES.length} files, ${(bytes / 1e6).toFixed(0)} MB in ${secs.toFixed(1)} s (${(bytes / 1e6 / secs).toFixed(0)} MB/s)`);
    const files = await Promise.all(FILES.map(async ([name, bpf]) => ({ bpf, file: await (await dir.getFileHandle(name)).getFile() })));
    if (ctx) {
      await Promise.race([ctx.resume().catch(() => {}), sleep(1000)]);
      if (ctx.state !== "running") { await ctx.close().catch(() => {}); ctx = null; }
    }
    const clock = ctx ? `audio at ${ctx.sampleRate} Hz` : audio ? "timer (the audio did not start)" : "timer";
    // Each run starts after the last one's read-ahead, so no run reads
    // what the one before read.  A short file starts again at 0.
    let from = 0;
    for (const tempo of [1, cfg.max_tempo]) {
      for (const block of [0, cfg.block_ms]) {
        if (from + (cfg.play_s * tempo + cfg.buffer_s + 1) * RATE > frames) from = 0;
        const ahead = new Ahead(files, frames, cfg);
        const r = await play(ahead, ctx, from, cfg.play_s, tempo, block, cfg.block_every_s, cfg.frames_per_buffer);
        const with_block = block > 0 ? `, ${block} ms block every ${cfg.block_every_s} s, blocks ${r.blocks}` : "";
        line(`playback: tempo ${tempo}${with_block}: underruns ${r.under}, ` +
          `late buffers ${r.late} (buffers of ${(r.period * 1000).toFixed(0)} ms that started after their time), ` +
          `${r.played.toFixed(1)} s played from ${(from / RATE).toFixed(0)} s, clock ${clock}` +
          (r.interrupted ? `, interrupted (the audio context is ${ctx.state})` : "") +
          (ahead.failed ? `, read error ${ahead.failed}` : ""));
        line(`reads: tempo ${tempo}${with_block}: one second of ${FILES.length} files: ` +
          `${st_times_text(ahead.times)}, ${ahead.times.length} reads`);
        from = r.end + ahead.cap;
      }
    }
    const ahead = new Ahead(files, frames, cfg);
    const seeks = [];
    for (let i = 0; i < cfg.seeks && !ahead.failed; i++) {
      // Spread over the song, so each target is outside the buffer.  No
      // read is in flight when the clock starts.
      const to = Math.floor(((i * 7919) % cfg.seeks) / cfg.seeks * Math.max(1, frames - RATE));
      await ahead.idle();
      const t0 = performance.now();
      ahead.seek(to);
      await ahead.until(() => ahead.end > to);
      if (!ahead.failed) seeks.push(performance.now() - t0);
    }
    line(`seeks: to the first frame of the new position: ${st_times_text(seeks)}, ${seeks.length} seeks` +
      (ahead.failed ? `, read error ${ahead.failed}` : ""));
    const c = await concurrent(dir, cfg.spy);
    line(`concurrent: reads of the last byte while a worker writes: compared ${c.right + c.wrong} ` +
      `(right ${c.right}, wrong ${c.wrong}), failed ${c.failed} (${c.errors}), size grew ${c.grew}, ` +
      `worker ${c.worker}, ${(c.bytes / 1e6).toFixed(0)} MB written` +
      (cfg.spy ? `, worker writes [${c.record}]` : ""));
  } finally {
    if (ctx) await ctx.close().catch(() => {});
    try { await remove_scratch(); }
    catch (e) { line(`cleanup: the scratch folder stays (${(e && e.name) || e})`); }
  }
  line("done");
}
"#)]
extern "C" {
    fn st_cfg(
        minutes: f64,
        play_s: f64,
        seeks: u32,
        max_tempo: f32,
        frames_per_buffer: u32,
    ) -> JsValue;
    fn st_cfg_set(cfg: &JsValue, key: &str, value: JsValue);
    fn st_query_cfg(search: &str, max_tempo: f32, frames_per_buffer: u32) -> JsValue;
    fn st_audio() -> JsValue;
    #[cfg(test)]
    fn st_times_text(times: &js_sys::Array) -> String;
    #[wasm_bindgen(catch)]
    async fn st_run(
        cfg: &JsValue,
        audio: &JsValue,
        line: &Closure<dyn FnMut(String)>,
    ) -> Result<JsValue, JsValue>;
}

/// What a run measures, and how long it takes.
pub struct Settings(JsValue);

impl Settings {
    /// The settings in the page address, if it asks for the test.
    pub fn from_url() -> Option<Self> {
        let search = web_sys::window()?.location().search().ok()?;
        Self::from_query(&search)
    }

    /// The settings in a page address's query, such as
    /// `?streamtest&st_minutes=5`, or `None` without `streamtest`.
    pub fn from_query(search: &str) -> Option<Self> {
        let cfg = st_query_cfg(
            search,
            engine::params::MAX_TEMPO,
            engine::slot::WEB_BUFFER_FRAMES,
        );
        (!cfg.is_null()).then_some(Self(cfg))
    }

    /// Files of `minutes`, `play_s` seconds of each playback run and
    /// `seeks` seeks.
    pub fn new(minutes: f64, play_s: f64, seeks: u32) -> Self {
        Self(st_cfg(
            minutes,
            play_s,
            seeks,
            engine::params::MAX_TEMPO,
            engine::slot::WEB_BUFFER_FRAMES,
        ))
    }

    /// Sets one more field of the settings, such as `buffer_s`.
    pub fn with(self, key: &str, value: impl Into<JsValue>) -> Self {
        st_cfg_set(&self.0, key, value.into());
        self
    }
}

/// An audio context, made and resumed at once.  Call it in a tap's
/// handler: Safari starts audio only there.  Null if the page cannot make
/// one.
pub fn audio() -> JsValue {
    st_audio()
}

/// Runs the test, on `audio` (from [`audio`], or null for a timer), and
/// gives each result line, without the prefix, to `line`.  The last line
/// is "done".
pub async fn run(
    settings: &Settings,
    audio: &JsValue,
    line: impl FnMut(String) + 'static,
) -> Result<(), String> {
    let line: Closure<dyn FnMut(String)> = Closure::new(line);
    st_run(&settings.0, audio, &line)
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
/// last one in `status`.  `audio` comes from [`audio`], in the tap.
pub fn start(
    settings: Settings,
    audio: JsValue,
    status: Rc<RefCell<Status>>,
    ctx: eframe::egui::Context,
) {
    status.borrow_mut().running = true;
    wasm_bindgen_futures::spawn_local(async move {
        let shown = status.clone();
        let repaint = ctx.clone();
        let result = run(&settings, &audio, move |text| {
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

/// Browser tests (`cargo run -p xtask -- web-test`).  `xtask web-test`
/// starts Chrome with no user-gesture rule for audio, so [`audio`] runs
/// there.  Safari keeps audio suspended while macOS reports the user idle
/// (`docs/references/browser-quirks.md`), so the tests check the audio
/// clock in Chrome only.
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
export function st_cfg_get(cfg, key) { return cfg[key]; }
export function st_in_chrome() { return navigator.userAgent.includes("Chrome/"); }
export function st_suspend_later(ctx, ms) { if (ctx) setTimeout(() => ctx.suspend(), ms); }
export async function st_scratch_exists() {
  try { await (await navigator.storage.getDirectory()).getDirectoryHandle("streamtest"); return true; }
  catch (e) { return false; }
}
"#)]
    extern "C" {
        fn st_spy_on();
        fn st_spy_off() -> String;
        fn st_cfg_get(cfg: &JsValue, key: &str) -> JsValue;
        fn st_in_chrome() -> bool;
        fn st_suspend_later(ctx: &JsValue, ms: u32);
        async fn st_scratch_exists() -> JsValue;
    }

    fn field(s: &Settings, key: &str) -> Option<f64> {
        st_cfg_get(&s.0, key).as_f64()
    }

    /// Runs the test and returns its lines.
    async fn lines_of(settings: &Settings, audio: &JsValue) -> Vec<String> {
        let lines = Rc::new(RefCell::new(Vec::new()));
        let seen = lines.clone();
        run(settings, audio, move |l| seen.borrow_mut().push(l))
            .await
            .expect("the spike runs");
        lines.take()
    }

    fn lines_with<'a>(lines: &'a [String], prefix: &str) -> Vec<&'a String> {
        lines.iter().filter(|l| l.starts_with(prefix)).collect()
    }

    /// The number after `label` in `line`, such as 3 in "underruns 3,".
    fn number(line: &str, label: &str) -> f64 {
        let rest = line
            .split(label)
            .nth(1)
            .unwrap_or_else(|| panic!("no {label:?} in {line}"));
        let digits: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        digits
            .parse()
            .unwrap_or_else(|_| panic!("no number after {label:?} in {line}"))
    }

    /// The spike with a short song: each measure logs its line, fast reads
    /// give no underruns, a block longer than two buffers makes late
    /// buffers, every write gets a whole array (MEM-5), and no file is
    /// left.  Chrome runs it on the audio clock.
    #[wasm_bindgen_test]
    async fn the_spike_logs_each_measure_and_leaves_no_files() {
        // 0.2 minutes: 19 MB in blocks of 1 MiB, so each file has full
        // blocks and a shorter last one.
        let settings = Settings::new(0.2, 1.0, 3)
            .with("block_bytes", 1 << 20)
            .with("block_ms", 150)
            .with("block_every_s", 0.25)
            .with("spy", true);
        st_spy_on();
        let lines = lines_of(&settings, &audio()).await;
        let writes = st_spy_off();
        assert_eq!(
            lines_with(&lines, "write: 8 files, ").len(),
            1,
            "{lines:#?}"
        );
        let playback = lines_with(&lines, "playback: tempo ");
        assert_eq!(playback.len(), 4, "{lines:#?}");
        assert!(lines.iter().any(|l| l.starts_with("playback: tempo 1.5")));
        for l in &playback {
            assert_eq!(number(l, "underruns "), 0.0, "{l}");
            assert!(
                !l.contains("read error") && !l.contains(" 0.0 s played"),
                "{l}"
            );
            if st_in_chrome() {
                assert!(
                    l.contains("clock audio at "),
                    "web-test runs Chrome with audio allowed: {l}"
                );
            } else if l.contains("clock timer") {
                console_log!("skipped: the audio context is suspended");
            }
        }
        for l in &playback {
            if l.contains(" ms block every ") {
                assert!(number(l, "blocks ") > 0.0, "no block ran: {l}");
                assert!(number(l, "late buffers ") > 0.0, "{l}");
            } else {
                assert_eq!(number(l, "late buffers "), 0.0, "{l}");
            }
        }
        assert_eq!(lines_with(&lines, "reads: tempo ").len(), 4, "{lines:#?}");
        let seeks = lines_with(&lines, "seeks: ")[0];
        assert!(seeks.ends_with(", 3 seeks"), "{seeks}");
        let concurrent = lines_with(&lines, "concurrent: ")[0];
        assert!(concurrent.contains("worker done"), "{concurrent}");
        assert!(
            number(concurrent, "compared ") + number(concurrent, "failed ") > 0.0,
            "no read was tried: {concurrent}"
        );
        assert_eq!(number(concurrent, "wrong "), 0.0, "{concurrent}");
        let worker_writes = concurrent
            .split("worker writes [")
            .nth(1)
            .expect("the worker's record");
        assert!(worker_writes.starts_with("whole"), "{concurrent}");
        assert!(!worker_writes.contains("view"), "{concurrent}");
        assert_eq!(lines.last().map(String::as_str), Some("done"));
        let writes: Vec<&str> = writes.split(' ').collect();
        assert!(
            writes.len() > 8,
            "MEM-5: each file needs full blocks: {writes:?}"
        );
        assert!(writes.iter().all(|w| *w == "whole"), "MEM-5: {writes:?}");
        assert_eq!(st_scratch_exists().await.as_bool(), Some(false));
    }

    /// On the timer clock, reads that take 400 ms show in the read and seek
    /// times, and they give underruns.  Each buffer has the page's frames
    /// (about 70 ms at 44.1 kHz).  A block between one and two buffers
    /// (120 ms) makes buffers late, as on cpal's schedule.
    #[wasm_bindgen_test]
    async fn slow_reads_underrun_and_long_blocks_make_late_buffers() {
        // Each run plays 1.2 s through a buffer of 1 s, so it needs a read.
        let settings = Settings::new(0.1, 1.2, 2)
            .with("buffer_s", 1)
            .with("read_delay_ms", 400)
            .with("block_ms", 120)
            .with("block_every_s", 0.2);
        let lines = lines_of(&settings, &JsValue::NULL).await;
        let playback = lines_with(&lines, "playback: tempo ");
        assert_eq!(playback.len(), 4, "{lines:#?}");
        for l in &playback {
            assert!(l.contains("clock timer"), "{l}");
            assert!(l.contains("(buffers of 70 ms "), "{l}");
            assert!(number(l, "underruns ") > 0.0, "{l}");
        }
        for l in &playback {
            let late = number(l, "late buffers ");
            if l.contains(" ms block every ") {
                assert!(late > 0.0, "{l}");
            } else {
                assert_eq!(late, 0.0, "{l}");
            }
        }
        for l in lines_with(&lines, "reads: tempo ") {
            assert!(number(l, "p50 ") >= 400.0, "{l}");
        }
        let seeks = lines_with(&lines, "seeks: ")[0];
        assert!(number(seeks, "p50 ") >= 400.0, "{seeks}");
        assert_eq!(st_scratch_exists().await.as_bool(), Some(false));
    }

    /// The times of a line are nearest-rank percentiles of the sorted
    /// numbers, not of the text.
    #[wasm_bindgen_test]
    fn times_are_percentiles_of_the_numbers() {
        let one_to_100: js_sys::Array = (1..=100).map(JsValue::from).collect();
        assert_eq!(
            st_times_text(&one_to_100),
            "p50 50.0 ms, p99 99.0 ms, max 100.0 ms"
        );
        let unsorted: js_sys::Array = [100, 9, 20].into_iter().map(JsValue::from).collect();
        assert_eq!(
            st_times_text(&unsorted),
            "p50 20.0 ms, p99 100.0 ms, max 100.0 ms"
        );
        assert_eq!(
            st_times_text(&js_sys::Array::new()),
            "p50 - ms, p99 - ms, max - ms"
        );
    }

    /// When the audio context stops during playback, the run says so and
    /// goes on to the end, not waiting for 'ended' events.
    #[wasm_bindgen_test]
    async fn a_stopped_audio_context_interrupts_the_playback() {
        let settings = Settings::new(0.2, 3.0, 1);
        let audio = audio();
        st_suspend_later(&audio, 1000);
        let lines = lines_of(&settings, &audio).await;
        assert_eq!(lines.last().map(String::as_str), Some("done"));
        let first = lines_with(&lines, "playback: tempo ")[0];
        if first.contains("clock timer") {
            assert!(
                !st_in_chrome(),
                "web-test runs Chrome with audio allowed: {first}"
            );
            console_log!("skipped: the audio context is suspended");
            return;
        }
        assert!(
            first.contains(", interrupted (the audio context is suspended)"),
            "{first}"
        );
        let played: f64 = first
            .split(" s played")
            .next()
            .and_then(|head| head.rsplit(", ").next())
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("no seconds played in {first}"));
        assert!(played < 3.0, "the run went on after the stop: {first}");
    }

    /// A failed read ends the waits: the run reports it, finishes and
    /// removes its files.
    #[wasm_bindgen_test]
    async fn a_failed_read_ends_the_run_cleanly() {
        let settings = Settings::new(0.05, 0.2, 5).with("fail_read_at", 2);
        let lines = lines_of(&settings, &JsValue::NULL).await;
        let seeks = lines_with(&lines, "seeks: ")[0];
        assert!(seeks.ends_with(", read error NotReadableError"), "{seeks}");
        assert_eq!(lines.last().map(String::as_str), Some("done"));
        assert_eq!(st_scratch_exists().await.as_bool(), Some(false));
    }

    /// `?streamtest` turns the test on, with defaults that its parameters
    /// change; a value that is not a positive number keeps the default.
    /// The top tempo is the player's, and the buffer is the page's output
    /// buffer.
    #[wasm_bindgen_test]
    fn the_page_address_sets_the_test() {
        assert!(Settings::from_query("").is_none());
        assert!(Settings::from_query("?bench").is_none());
        let defaults = Settings::from_query("?streamtest").expect("on");
        let got =
            |s: &Settings| ["minutes", "play_s", "seeks"].map(|k| field(s, k).expect("a number"));
        assert_eq!(got(&defaults), [30.0, 300.0, 50.0]);
        assert_eq!(
            field(&defaults, "max_tempo"),
            Some(f64::from(engine::params::MAX_TEMPO))
        );
        for settings in [&defaults, &Settings::new(1.0, 1.0, 1)] {
            assert_eq!(
                field(settings, "frames_per_buffer"),
                Some(f64::from(engine::slot::WEB_BUFFER_FRAMES)),
                "the page's output buffer"
            );
        }
        let set =
            Settings::from_query("?streamtest&st_minutes=0.5&st_play=20&st_seeks=7").expect("on");
        assert_eq!(got(&set), [0.5, 20.0, 7.0]);
        let bad = Settings::from_query("?st_minutes=lloyd&streamtest&st_play=-1").expect("on");
        assert_eq!(got(&bad), [30.0, 300.0, 50.0]);
    }
}
