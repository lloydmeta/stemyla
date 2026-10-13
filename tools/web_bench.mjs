// Headless smoke test of the in-app matmul benchmark (`?bench`): every
// kernel compiles in Chromium and runs on every model shape.  No model
// needed.  SwiftShader is slow, so the default does one dispatch each.
//
// Usage: node tools/web_bench.mjs <out_dir> [query]   (DIST defaults to app/dist)
// (query defaults to "bench&bench_quick=1&mm=s64")
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';

const root = process.env.STEMS_ROOT ?? path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
const out = process.argv[2] ?? '.';
const query = process.argv[3] ?? "bench&bench_quick=1&mm=s64";
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

const server = await startServer(root, process.env.DIST ?? 'app/dist', []);
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
  if (/bench finished|WebGPU error|separator: .*(failed|stopped)/.test(t)) verdict = t;
});
page.on('pageerror', (e) => note(`PAGE ERROR: ${e.message}`));
await page.goto(`${server.base}/index.html?${query}`);
for (let i = 0; i < 4 * 3600 && !verdict; i++) await page.waitForTimeout(1000);
note(`verdict: ${verdict}`);
fs.writeFileSync(path.join(out, 'web_bench.log'), log.join('\n') + '\n');
await browser.close();
server.close();
process.exit(verdict && /bench finished/.test(verdict) && !log.some((l) => /WebGPU error/.test(l)) ? 0 : 1);
