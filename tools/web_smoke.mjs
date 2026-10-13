// M7 browser smoke test: load the Trunk build in headless Chromium, load the
// synthetic stems through the multi-file drop path, press Play, collect the
// console, and check the Web Audio context is running.
//
// Usage: node tools/web_smoke.mjs <base-url> <out-dir>
// The server must serve app/dist at <base-url>/ and audio/synth at <base-url>/stems/.
import { chromium } from 'playwright';
import fs from 'node:fs';

const base = process.argv[2] ?? 'http://127.0.0.1:8765';
const out = process.argv[3] ?? '.';
const W = 1100, H = 600;
const log = [];
const t0 = Date.now();
const note = (m) => { const l = `[${((Date.now() - t0) / 1000).toFixed(1)}s] ${m}`; log.push(l); console.log(l); };

const browser = await chromium.launch({
  executablePath: '/opt/pw-browsers/chromium-1194/chrome-linux/chrome',
  args: ['--autoplay-policy=no-user-gesture-required', '--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
});
const page = await browser.newPage({ viewport: { width: W, height: H } });
page.on('console', (m) => note(`console.${m.type()}: ${m.text()}`));
page.on('pageerror', (e) => note(`PAGE ERROR: ${e.message}`));

// Track every AudioContext the page creates.
await page.addInitScript(() => {
  const Orig = window.AudioContext;
  window.__ctxs = [];
  window.AudioContext = function (...a) { const c = new Orig(...a); window.__ctxs.push(c); return c; };
  window.AudioContext.prototype = Orig.prototype;
});

await page.goto(`${base}/index.html`);
await page.waitForTimeout(3000);
await page.screenshot({ path: `${out}/web_empty.png` });
note('loaded; webgl2: ' + await page.evaluate(() => !!document.createElement('canvas').getContext('webgl2')));

// Multi-file drop: fetch the stem WAVs, wrap them as File objects, and
// dispatch dragenter/dragover/drop on the canvas, as a user's drop would.
const names = ['drums', 'bass', 'guitar', 'keys', 'lead'];
const dropped = await page.evaluate(async (names) => {
  const dt = new DataTransfer();
  for (const n of names) {
    const b = await (await fetch(`stems/${n}.wav`)).blob();
    dt.items.add(new File([b], `${n}.wav`, { type: 'audio/wav' }));
  }
  const c = document.getElementById('stem_explorer');
  const r = c.getBoundingClientRect();
  const opts = { bubbles: true, cancelable: true, dataTransfer: dt, clientX: r.width / 2, clientY: r.height / 2 };
  for (const t of ['dragenter', 'dragover', 'drop']) c.dispatchEvent(new DragEvent(t, opts));
  return dt.files.length;
}, names);
note(`dispatched drop with ${dropped} files`);
await page.waitForTimeout(8000);
await page.screenshot({ path: `${out}/web_loaded.png` });

// Press Play (second button in the transport bar; position from the native snapshot).
await page.mouse.click(105, 15);
note('clicked Play');
await page.waitForTimeout(5000);
const ctx = await page.evaluate(() => window.__ctxs.map((c) => ({ state: c.state, sampleRate: c.sampleRate, currentTime: c.currentTime, baseLatency: c.baseLatency })));
note('audio contexts: ' + JSON.stringify(ctx));

// Slow down to 75%: click the tempo slider a quarter of the way along its
// 50..150% track (x 213..303 in this layout).
await page.mouse.click(236, 15);
note('clicked tempo slider at 75%');
await page.waitForTimeout(8500);
await page.screenshot({ path: `${out}/web_playing.png` });

const errors = log.filter((l) => /PAGE ERROR|console\.error/.test(l));
note(`errors: ${errors.length}`);
fs.writeFileSync(`${out}/web_smoke.log`, log.join('\n') + '\n');
await browser.close();
