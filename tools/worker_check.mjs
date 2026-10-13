// In-browser check of the stem worker (BS-RoFormer on WebGPU) against
// PyTorch.  Serves app/dist and the exported weights with `xtask serve`, starts the worker in
// headless Chromium (WebGPU on SwiftShader, so on the CPU), loads the model,
// separates a 0.5 s clip with 0.37 s chunks and compares every stem with
// bs-roformer-infer's demix_track output (tools/roformer_export.py).
//
// Usage: node tools/worker_check.mjs <out.json> [weights=f16|f32]   (DIST defaults to app/dist)
// SwiftShader is slow (about 0.25 GFLOP/s), so the step limit is raised to 600 s.
// mm=s64 skips the per-GPU kernel timing, which takes minutes on SwiftShader.
// SKIP_SEPARATE=1 only loads the model and checks the browser-storage cache;
// DOWNLOAD=1 loads it through the worker's download path (served locally).
import { chromium } from 'playwright';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const root = process.env.STEMS_ROOT ?? path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
const out = process.argv[2] ?? 'worker_check.json';
const precision = process.argv[3] ?? 'f16';
const t0 = Date.now();

// Static files come from the project's own Rust server (`xtask serve`).
async function startServer(root, dist, mounts) {
  const { spawn } = await import('node:child_process');
  const args = ['run', '-q', '-p', 'xtask', '--release', '--', 'serve', dist, '--port', '0'];
  for (const [prefix, dir] of mounts) args.push('--mount', `${prefix}=${dir}`);
  const child = spawn('cargo', args, { cwd: root, stdio: ['ignore', 'pipe', 'inherit'] });
  const base = await new Promise((resolve, reject) => {
    let out = '';
    child.stdout.on('data', (d) => {
      out += d;
      const m = out.match(/at (http:\/\/[^\s]+)\//);
      if (m) resolve(m[1]);
    });
    child.on('exit', (c) => reject(new Error(`xtask serve exited with ${c}`)));
  });
  return { base, close: () => child.kill() };
}
const note = (m) => console.log(`[${((Date.now() - t0) / 1000).toFixed(1)}s] ${m}`);

// A blank page (not the app, which would start its own worker).
const blankDir = fs.mkdtempSync(path.join(os.tmpdir(), 'worker-check-'));
fs.writeFileSync(path.join(blankDir, 'blank.html'), '<!doctype html><title>worker check</title>');
const server = await startServer(root, process.env.DIST ?? 'app/dist',
  [['/weights', 'third_party/roformer'], ['/check', blankDir]]);
const base = server.base;

const browser = await chromium.launch({
  executablePath: '/opt/pw-browsers/chromium-1194/chrome-linux/chrome',
  ignoreDefaultArgs: ['--disable-gpu'],
  args: ['--enable-unsafe-webgpu', '--enable-features=Vulkan', '--use-vulkan=swiftshader',
         '--use-webgpu-adapter=swiftshader', '--use-angle=swiftshader', '--disable-vulkan-surface', ...(process.env.CHROME_LOG ? ['--enable-logging=stderr', '--v=0'] : [])],
});
const page = await browser.newPage();
page.on('console', (m) => note(`console.${m.type()}: ${m.text()}`));
page.on('pageerror', (e) => note(`PAGE ERROR: ${e.message}`));
await page.exposeFunction('note', note);
await page.goto(`${base}/check/blank.html`);

const result = await page.evaluate(async ({ precision, skipSeparate, download }) => {
  const start = (settings) => new Promise((resolve, reject) => {
    const w = new Worker('/stem-worker_loader.js', { type: 'module' });
    w.onerror = (e) => { window.note('worker error: ' + e.message); reject(new Error('worker error: ' + e.message)); };
    w.onmessage = (e) => { if (e.data.type === 'alive') resolve(w); };
  });
  const next = (w, types) => new Promise((resolve) => {
    w.onmessage = (e) => {
      if (e.data.type === 'progress') window.note(`progress ${e.data.done}/${e.data.total} at ${e.data.secs.toFixed(1)} s`);
      if (types.includes(e.data.type)) resolve(e.data);
    };
  });
  const r = { precision };
  const gpu = await navigator.gpu.requestAdapter();
  r.adapter = gpu ? `${gpu.info.vendor} ${gpu.info.architecture}` : 'none';

  let w = await start();
  let t = performance.now();
  let m;
  if (download) {
    // The "Download model" path, from this server instead of Hugging Face.
    w.postMessage({ type: 'download', mm: 's64', model_url: `/weights/bs_roformer_sw.${precision}.safetensors` });
    m = await new Promise((resolve) => {
      w.onmessage = (e) => {
        if (e.data.type === 'download') window.note(`download ${e.data.done} of ${e.data.total} bytes`);
        if (e.data.type === 'model' || e.data.type === 'error') resolve(e.data);
      };
    });
  } else {
    const weights = new Uint8Array(await (await fetch(`/weights/bs_roformer_sw.${precision}.safetensors`)).arrayBuffer());
    w.postMessage({ type: 'model', mm: 's64', bytes: weights }, [weights.buffer]);
    m = await next(w, ['model', 'error']);
  }
  r.model = m; r.model_ms = performance.now() - t;
  window.note(`model: ${JSON.stringify(m)}`);
  if (!m.ok) return r;

  if (skipSeparate) return await cachedCheck(w, r);
  const clip = new Float32Array(await (await fetch('/weights/web/clip.f32')).arrayBuffer());
  const ref = new Float32Array(await (await fetch('/weights/web/clip_ref.f32')).arrayBuffer());
  const n = clip.length / 2;
  const stems = {};
  t = performance.now();
  w.postMessage({ type: 'separate', left: clip.slice(0, n), right: clip.slice(n), chunk: 32 * 512, overlap: 2, debug: true, step_limit: 600 });
  await new Promise((resolve) => {
    w.onmessage = (e) => {
      const d = e.data;
      if (d.type === 'progress') window.note(`progress ${d.done}/${d.total} at ${d.secs.toFixed(1)} s`);
      if (d.type === 'log') window.note(`worker: ${d.text}`);
      if (d.type === 'stem') stems[d.name] = [d.left, d.right];
      if (d.type === 'done' || d.type === 'error') { r.end = d; resolve(); }
    };
  });
  r.separate_ms = performance.now() - t;
  const names = ['bass', 'drums', 'other', 'vocals', 'guitar', 'piano'];
  let mixE = 0;
  for (const v of clip) mixE += v * v;
  r.stems = {};
  names.forEach((name, s) => {
    if (!stems[name]) { r.stems[name] = 'missing'; return; }
    let e = 0, err = 0;
    for (let c = 0; c < 2; c++) {
      const want = ref.subarray((s * 2 + c) * n, (s * 2 + c + 1) * n);
      const got = stems[name][c];
      for (let i = 0; i < n; i++) { e += want[i] * want[i]; err += (got[i] - want[i]) ** 2; }
    }
    r.stems[name] = { vs_stem_db: 10 * Math.log10(e / err), vs_mix_db: 10 * Math.log10(mixE / err), share_pct: 100 * e / mixE };
  });

  return await cachedCheck(w, r);

  // A fresh worker should find the model in the browser's private storage.
  async function cachedCheck(w, r) {
    w.terminate();
    w = await start();
    const t = performance.now();
    w.postMessage({ type: 'init', mm: 's64' });
    r.cached = await next(w, ['model', 'error']);
    r.cached_ms = performance.now() - t;
    w.terminate();
    return r;
  }
}, { precision, skipSeparate: !!process.env.SKIP_SEPARATE, download: !!process.env.DOWNLOAD });

note(JSON.stringify(result, null, 1));
fs.writeFileSync(out, JSON.stringify(result, null, 1));
await browser.close();
server.close();
