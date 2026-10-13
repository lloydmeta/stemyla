// Headless check that playback starts after a click when the page's audio
// output was created before any user gesture: the case of a song restored
// from browser storage at page load.  Browsers then create the Web Audio
// context suspended.  Chromium starts such a context by itself on the first
// click, but Safari resumes it only when the page calls resume() inside a
// click handler, so this test makes Chromium behave like Safari: every
// AudioContext starts suspended and resume() works only during a user
// activation.
//
// Usage: node tools/web_audio_resume.mjs <song.mp3>   (DIST defaults to app/dist)
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';

const root = process.env.STEMS_ROOT ?? path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
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
const browser = await chromium.launch({
  executablePath: '/opt/pw-browsers/chromium-1194/chrome-linux/chrome',
  args: ['--autoplay-policy=user-gesture-required'],
});
const page = await browser.newPage({ viewport: { width: 1100, height: 700 } });
await page.addInitScript(() => {
  const Real = window.AudioContext;
  window.AudioContext = function (...args) {
    const ctx = new Real(...args);
    ctx.suspend();
    const resume = ctx.resume.bind(ctx);
    ctx.resume = () => (navigator.userActivation.isActive ? resume() : Promise.resolve());
    return ctx;
  };
  window.AudioContext.prototype = Real.prototype;
});
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

// Store a song (the drop is not a user gesture), then reload so the app
// restores it and opens the audio output at load, before any click.
await page.goto(`${server.base}/index.html`);
await page.waitForTimeout(3000);
const name = path.basename(song);
await page.evaluate(async ({ url, name }) => {
  const bytes = await (await fetch(url)).arrayBuffer();
  const dt = new DataTransfer();
  dt.items.add(new File([bytes], name, { type: 'audio/mpeg' }));
  const c = document.querySelector('canvas');
  const r = c.getBoundingClientRect();
  const opts = { bubbles: true, cancelable: true, dataTransfer: dt, clientX: r.width / 2, clientY: r.height / 2 };
  for (const t of ['dragenter', 'dragover', 'drop']) c.dispatchEvent(new DragEvent(t, opts));
}, { url: `/input/${name}`, name });
if (!(await waitLog(/song kept in browser storage/, 60))) fail('song not stored');
log.length = 0;
await page.reload();
if (!(await waitLog(/Restored ".*" from this browser/, 60))) fail('song not restored');
await page.waitForTimeout(3000);
const stats = () => log.filter((l) => /audio stats: \d+ callbacks/.test(l)).map((l) => +l.match(/audio stats: (\d+) callbacks/)[1]);
const heads = () => log.filter((l) => /playhead \d+/.test(l)).map((l) => +l.match(/playhead (\d+)/)[1]);
note(`before the click: ${stats().length} audio stats lines with callbacks, playhead ${heads().at(-1) ?? 0}`);

// "Play" in the transport bar (without a model: Demo song, Download model,
// Load model file, Storage, About, Play).
await page.mouse.click(Number(process.env.PLAY_X ?? 508), 15);
await page.waitForTimeout(5000);
const head = heads().at(-1) ?? 0;
note(`after the click: playhead ${head}`);
if (head < 44100) fail('the playhead did not move (no audio callbacks after the click)');
await browser.close();
server.close();
note(process.exitCode ? 'audio resume check FAILED' : 'audio resume check passed');
