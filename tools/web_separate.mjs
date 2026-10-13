// End-to-end browser test of in-app separation: open the web app in headless
// Chromium (WebGPU on SwiftShader), drop a short clip as the song, choose
// the model through the "Load model…" file picker, press "Separate 6
// stems", wait for the lanes, take screenshots.
//
// Small chunks keep SwiftShader's CPU-bound WebGPU within minutes; the
// numeric agreement with PyTorch is checked separately by worker_check.mjs.
//
// Usage: DIST=app/dist node tools/web_separate.mjs <out_dir> [query]
// (query defaults to "chunk=16384&overlap=1")
import { chromium } from 'playwright';
import os from 'node:os';
import fs from 'node:fs';
import path from 'node:path';

const root = process.env.STEMS_ROOT ?? path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
const out = process.argv[2] ?? '.';
const query = process.argv[3] ?? 'chunk=16384&overlap=1';
const weights = path.join(root, 'third_party/roformer/bs_roformer_sw.f16.safetensors');
const log = [];
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
const note = (m) => { const l = `[${((Date.now() - t0) / 1000).toFixed(1)}s] ${m}`; log.push(l); console.log(l); };

// The 0.5 s clip from roformer_export.py as a float WAV.
function wav(f32, channels, rate) {
  const n = f32.length / channels;
  const data = Buffer.alloc(n * channels * 4);
  for (let i = 0; i < n; i++) for (let c = 0; c < channels; c++) data.writeFloatLE(f32[c * n + i], (i * channels + c) * 4);
  const h = Buffer.alloc(44);
  h.write('RIFF', 0); h.writeUInt32LE(36 + data.length, 4); h.write('WAVE', 8);
  h.write('fmt ', 12); h.writeUInt32LE(16, 16); h.writeUInt16LE(3, 20); h.writeUInt16LE(channels, 22);
  h.writeUInt32LE(rate, 24); h.writeUInt32LE(rate * channels * 4, 28); h.writeUInt16LE(channels * 4, 32); h.writeUInt16LE(32, 34);
  h.write('data', 36); h.writeUInt32LE(data.length, 40);
  return Buffer.concat([h, data]);
}
const raw = fs.readFileSync(path.join(root, 'third_party/roformer/web/clip.f32'));
const clipWav = wav(new Float32Array(raw.buffer, raw.byteOffset, raw.length / 4), 2, 44100);

const clipDir = fs.mkdtempSync(path.join(os.tmpdir(), 'web-separate-'));
fs.writeFileSync(path.join(clipDir, 'clip.wav'), clipWav);
const server = await startServer(root, process.env.DIST ?? 'app/dist', [['/test', clipDir]]);
const base = server.base;

const browser = await chromium.launch({
  executablePath: '/opt/pw-browsers/chromium-1194/chrome-linux/chrome',
  ignoreDefaultArgs: ['--disable-gpu'],
  args: ['--enable-unsafe-webgpu', '--enable-features=Vulkan', '--use-vulkan=swiftshader',
         '--use-webgpu-adapter=swiftshader', '--use-angle=swiftshader', '--disable-vulkan-surface',
         '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist', '--autoplay-policy=no-user-gesture-required'],
});
const W = 1100, H = 700;
const page = await browser.newPage({ viewport: { width: W, height: H } });
let modelText = null, separated = null;
page.on('console', (m) => {
  const t = m.text();
  note(`console.${m.type()}: ${t}`);
  const mm = t.match(/separator: (model loaded.*)/);
  if (mm) modelText = mm[1];
  if (/separator: \d+ stems in/.test(t)) separated = t;
});
page.on('pageerror', (e) => note(`PAGE ERROR: ${e.message}`));

await page.goto(`${base}/${process.env.PAGE ?? 'index.html'}?${query}`);
await page.waitForTimeout(4000);
await page.screenshot({ path: `${out}/web_sep_empty.png` });

// Drop the clip as the song.
await page.evaluate(async () => {
  const dt = new DataTransfer();
  const b = await (await fetch('test/clip.wav')).blob();
  dt.items.add(new File([b], 'clip.wav', { type: 'audio/wav' }));
  const c = document.getElementById('stem_explorer');
  const r = c.getBoundingClientRect();
  const opts = { bubbles: true, cancelable: true, dataTransfer: dt, clientX: r.width / 2, clientY: r.height / 2 };
  for (const t of ['dragenter', 'dragover', 'drop']) c.dispatchEvent(new DragEvent(t, opts));
});
await page.waitForTimeout(4000);
await page.screenshot({ path: `${out}/web_sep_song.png` });

// "Load model…" sits right of "Demo song" in the transport bar.
// egui acts on the click in its next frame; retry if no picker opened.
let chooser = null;
for (let attempt = 1; attempt <= 3 && !chooser; attempt++) {
  await page.mouse.move(Number(process.env.MODEL_X ?? 115), 15);
  await page.waitForTimeout(300);
  [chooser] = await Promise.all([
    page.waitForEvent('filechooser', { timeout: 10000 }).catch(() => null),
    page.mouse.click(Number(process.env.MODEL_X ?? 115), 15),
  ]);
  if (!chooser) note(`no file picker after click ${attempt}`);
}
if (!chooser) throw new Error('the Load model button did not open a file picker');
await chooser.setFiles(weights);
note('model file chosen');
for (let i = 0; i < 120 && !modelText; i++) await page.waitForTimeout(1000);
note(`model: ${modelText}`);
await page.waitForTimeout(1000);
await page.screenshot({ path: `${out}/web_sep_model.png` });

// The same slot now holds "Separate 6 stems".
await page.mouse.click(Number(process.env.SEPARATE_X ?? 125), 15);
note('clicked Separate');
await page.waitForTimeout(20000);
await page.screenshot({ path: `${out}/web_sep_running.png` });
for (let i = 0; i < 3600 && !separated; i++) await page.waitForTimeout(1000);
note(`separated: ${separated}`);
await page.waitForTimeout(2000);
await page.screenshot({ path: `${out}/web_separated.png` });

const errors = log.filter((l) => /PAGE ERROR|console\.error/.test(l) && !/favicon|404/.test(l));
note(`errors: ${errors.length}`);
fs.writeFileSync(`${out}/web_separate.log`, log.join('\n') + '\n');
await browser.close();
server.close();
process.exit(separated && modelText ? 0 : 1);
