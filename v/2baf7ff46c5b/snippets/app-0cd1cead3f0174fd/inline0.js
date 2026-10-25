
const RATE = 44100;
const FRAMES_PER_BUFFER = 2048;
const SCRATCH = "streamtest";
// Name and bytes per stereo frame: 16-bit stems take 4, the f32 original 8.
const FILES = [["bass", 4], ["drums", 4], ["other", 4], ["vocals", 4], ["guitar", 4], ["piano", 4],
  ["original", 8], ["classic drums", 4]];

// `block_ms` is shorter than one buffer (about 43 to 46 ms), the slack
// that cpal has at each 'ended' event.  `read_delay_ms` and
// `fail_read_at` (the read of each read-ahead buffer that fails, from 1)
// are for tests.
export function st_cfg(minutes, play_s, seeks, max_tempo) {
  return { minutes, play_s, seeks, max_tempo, buffer_s: 10, block_ms: 30, block_every_s: 3, block_bytes: 8 << 20,
    read_delay_ms: 0, fail_read_at: 0, spy: false };
}

export function st_cfg_set(cfg, key, value) { cfg[key] = value; }

// The settings in a page address's query, or null without `streamtest`.
export function st_query_cfg(search, max_tempo) {
  const q = new URLSearchParams(search);
  if (!q.has("streamtest")) return null;
  const num = (k, d) => { const v = parseFloat(q.get(k)); return v > 0 ? v : d; };
  return st_cfg(num("st_minutes", 30), num("st_play", 300), Math.round(num("st_seeks", 50)), max_tempo);
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
async function play(ahead, ctx, from, seconds, tempo, block_ms, block_every_s) {
  ahead.seek(from);
  await ahead.until(() => ahead.end - ahead.start + RATE > ahead.cap || ahead.end >= ahead.frames);
  let under = 0, late = 0, buffers = 0, blocks = 0, pos = from, interrupted = false;
  const rate = ctx ? ctx.sampleRate : RATE;
  const step = Math.round((FRAMES_PER_BUFFER * RATE * tempo) / rate);
  const period = FRAMES_PER_BUFFER / rate;
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
      const silence = ctx.createBuffer(2, FRAMES_PER_BUFFER, rate);
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
        const r = await play(ahead, ctx, from, cfg.play_s, tempo, block, cfg.block_every_s);
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
