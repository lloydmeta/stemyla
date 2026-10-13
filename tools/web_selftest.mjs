// Headless check of the in-app GPU self-test (`?selftest`): serve the app
// and the reference files with `xtask serve`, load the model through the
// file picker, wait for the self-test's verdict, save the console log.
//
// Usage: node tools/web_selftest.mjs <out_dir> [query]   (DIST defaults to app/dist)
// (query defaults to "selftest=stages&step_limit=600&mm=s64": SwiftShader runs
// about 0.15 GFLOP/s, far too slow for the full 13 s chunk)
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';

const root = process.env.STEMS_ROOT ?? path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
const out = process.argv[2] ?? '.';
const query = process.argv[3] ?? 'selftest=stages&step_limit=600&mm=s64';
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

const server = await startServer(root, process.env.DIST ?? 'app/dist', [['/selftest', 'third_party/roformer/selftest']]);
const browser = await chromium.launch({
  executablePath: '/opt/pw-browsers/chromium-1194/chrome-linux/chrome',
  ignoreDefaultArgs: ['--disable-gpu'],
  args: ['--enable-unsafe-webgpu', '--enable-features=Vulkan', '--use-vulkan=swiftshader',
         '--use-webgpu-adapter=swiftshader', '--use-angle=swiftshader', '--disable-vulkan-surface',
         '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
});
const page = await browser.newPage({ viewport: { width: 1100, height: 700 } });
let verdict = null;
page.on('console', (m) => {
  const t = m.text();
  if (/GL Driver/.test(t)) return;
  note(`console.${m.type()}: ${t}`);
  if (/selftest (passed|FAILED)|Separation failed/.test(t)) verdict = t;
});
page.on('pageerror', (e) => note(`PAGE ERROR: ${e.message}`));
await page.goto(`${server.base}/index.html?${query}`);
await page.waitForTimeout(4000);
let chooser = null;
for (let attempt = 1; attempt <= 3 && !chooser; attempt++) {
  await page.mouse.move(115, 15);
  await page.waitForTimeout(300);
  [chooser] = await Promise.all([
    page.waitForEvent('filechooser', { timeout: 10000 }).catch(() => null),
    page.mouse.click(115, 15),
  ]);
}
if (!chooser) throw new Error('no file picker');
await chooser.setFiles(weights);
note('model file chosen');
for (let i = 0; i < 4 * 3600 && !verdict; i++) await page.waitForTimeout(1000);
note(`verdict: ${verdict}`);
fs.writeFileSync(path.join(out, 'web_selftest.log'), log.join('\n') + '\n');
await browser.close();
server.close();
process.exit(verdict && /passed/.test(verdict) ? 0 : 1);
