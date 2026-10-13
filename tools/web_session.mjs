// Headless check of the browser-local song session: drop an MP3 on the
// app, check it lands in browser storage (OPFS), reload and check the app
// restores it; then add two separated stems to the stored session by hand
// (16-bit WAVs, as the app writes them), reload and check the app restores
// the song with its stems.  No model or GPU work involved.
//
// Usage: node tools/web_session.mjs <song.mp3>   (DIST defaults to app/dist)
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';

const root = process.env.STEMS_ROOT ?? path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
const out = process.argv[2] ?? '.';
const query = process.argv[3] ?? "bench&bench_work=1";
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
const song = process.argv[2];
const server = await startServer(root, process.env.DIST ?? 'app/dist', [['/input', path.dirname(path.resolve(song))]]);
const browser = await chromium.launch({ executablePath: '/opt/pw-browsers/chromium-1194/chrome-linux/chrome' });
const page = await browser.newPage({ viewport: { width: 1100, height: 700 } });
page.on('console', (m) => note(`console.${m.type()}: ${m.text()}`));
page.on('pageerror', (e) => note(`PAGE ERROR: ${e.message}`));
const waitLog = async (re, secs) => {
  for (let i = 0; i < secs * 4; i++) {
    if (log.some((l) => re.test(l))) return true;
    await page.waitForTimeout(250);
  }
  return false;
};
const fail = (m) => { note(`FAIL: ${m}`); process.exitCode = 1; };

await page.goto(`${server.base}/index.html`);
await page.waitForTimeout(3000);
const name = path.basename(song);
// Drop the file on the canvas the way a browser does.
await page.evaluate(async ({ url, name }) => {
  const bytes = await (await fetch(url)).arrayBuffer();
  const dt = new DataTransfer();
  dt.items.add(new File([bytes], name, { type: 'audio/mpeg' }));
  const c = document.querySelector('canvas');
  const r = c.getBoundingClientRect();
  const opts = { bubbles: true, cancelable: true, dataTransfer: dt, clientX: r.width / 2, clientY: r.height / 2 };
  for (const t of ['dragenter', 'dragover', 'drop']) c.dispatchEvent(new DragEvent(t, opts));
}, { url: `/input/${name}`, name });
if (!(await waitLog(/song kept in browser storage as session (\d+)/, 60))) fail('song not stored');
const listing = async () => page.evaluate(async () => {
  const out = [];
  async function walk(d, p) {
    for await (const [n, h] of d.entries()) {
      if (h.kind === 'directory') await walk(h, `${p}${n}/`);
      else out.push(`${p}${n} ${(await h.getFile()).size}`);
    }
  }
  await walk(await navigator.storage.getDirectory(), '');
  return out.sort();
});
note(`storage after drop: ${JSON.stringify(await listing())}`);

log.length = 0;
await page.reload();
if (!(await waitLog(/Restored ".*" from this browser/, 60))) fail('song not restored after reload');

// Two stems into the stored session, as save_stems would write them.
await page.evaluate(async () => {
  const root = await navigator.storage.getDirectory();
  const sessions = await root.getDirectoryHandle('sessions');
  const id = (await (await (await sessions.getFileHandle('current')).getFile()).text()).trim();
  const dir = await sessions.getDirectoryHandle(id);
  const metaFile = await dir.getFileHandle('meta');
  const rate = 44100;
  const meta = (await (await metaFile.getFile()).text()).trim();
  const frames = 1000;
  const wav = (f) => {
    const b = new DataView(new ArrayBuffer(44 + frames * 4));
    const s = (o, t) => [...t].forEach((c, i) => b.setUint8(o + i, c.charCodeAt(0)));
    s(0, 'RIFF'); b.setUint32(4, 36 + frames * 4, true); s(8, 'WAVEfmt ');
    b.setUint32(16, 16, true); b.setUint16(20, 1, true); b.setUint16(22, 2, true);
    b.setUint32(24, rate, true); b.setUint32(28, rate * 4, true); b.setUint16(32, 4, true); b.setUint16(34, 16, true);
    s(36, 'data'); b.setUint32(40, frames * 4, true);
    for (let i = 0; i < frames * 2; i++) b.setInt16(44 + 2 * i, Math.round(8000 * Math.sin(i * f)), true);
    return b.buffer;
  };
  const stems = await dir.getDirectoryHandle('stems', { create: true });
  for (const [n, f] of [['guitar', 0.01], ['drums', 0.03]]) {
    const w = await (await stems.getFileHandle(`${n}.wav`, { create: true })).createWritable();
    await w.write(wav(f)); await w.close();
  }
  const w = await metaFile.createWritable();
  await w.write(meta + '\nstem\tguitar\nstem\tdrums\n'); await w.close();
});
log.length = 0;
await page.reload();
if (!(await waitLog(/Restored ".*" and its separated stems from this browser/, 60))) fail('stems not restored after reload');
note(`storage at the end: ${JSON.stringify(await listing())}`);
await browser.close();
server.close();
note(process.exitCode ? 'session check FAILED' : 'session check passed');
