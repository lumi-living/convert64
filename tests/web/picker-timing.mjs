// Method-picker thumbnail timing in the browser build (#260, #275; UX §6a).
//
//   (cd crates/convert64-app && trunk build --release)
//   node tests/web/picker-timing.mjs [dist dir] [picture]
//
// Needs only Node 22 and Google Chrome (CHROME=path to override), no npm packages, like
// restore-smoke.mjs. It serves the bundle and the picture (default: the `title` fixture)
// on a free port, starts headless Chrome at 1440 × 900 with a throwaway profile, drops
// the picture on the page, waits for the result, opens the method picker on the layer's
// recipe (a click on the Recipe tab's Method button) and waits for the app's timing log:
//
//   convert64: the visible method thumbnails (N rendered) in X ms (#275)
//   convert64: rendered 90 method thumbnails (Gradient) in X ms, Y ms of solving, … (#260)
//
// It prints both lines, the time from the click to each, and the machine's load
// (`os.loadavg()`). RUNS=n repeats the open n times (closing with Esc and changing the
// strength in between, so the cache does not serve the second open); SHOTS=dir saves a
// screenshot of each open. Exit code 0 when every open logged both lines.

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';

const here = path.dirname(new URL(import.meta.url).pathname);
const repo = path.resolve(here, '..', '..');
const dist = path.resolve(process.argv[2] || path.join(repo, 'crates/convert64-app/dist'));
const picture = path.resolve(process.argv[3] || path.join(repo, 'fixtures/title/source.png'));
const shots = process.env.SHOTS ? path.resolve(process.env.SHOTS) : null;
const runs = Number(process.env.RUNS || 1);
const chrome = process.env.CHROME || (process.platform === 'darwin'
  ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
  : 'google-chrome');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const W = 1440, H = 900;

if (!fs.existsSync(path.join(dist, 'index.html'))) {
  console.error(`no ${dist}/index.html: run (cd crates/convert64-app && trunk build --release)`);
  process.exit(2);
}

const types = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.png': 'image/png' };
const server = http.createServer((req, res) => {
  const url = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  const file = url === '/picture.png' ? picture : path.join(dist, url === '/' ? 'index.html' : url);
  if (!file.startsWith(dist) && file !== picture) { res.writeHead(403); return res.end(); }
  fs.readFile(file, (err, data) => {
    if (err) { res.writeHead(404); return res.end(); }
    res.writeHead(200, { 'content-type': types[path.extname(file)] || 'application/octet-stream' });
    res.end(data);
  });
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const httpPort = server.address().port;

const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'convert64-picker-'));
const browser = spawn(chrome, [
  '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
  '--no-first-run', '--no-default-browser-check', `--window-size=${W},${H}`, 'about:blank',
], { stdio: 'ignore' });

let failed = 0;
const finish = async code => {
  const exited = new Promise(r => browser.once('exit', r));
  try { browser.kill('SIGKILL'); } catch {}
  await Promise.race([exited, sleep(5000)]);
  server.close();
  try { fs.rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 }); } catch {}
  process.exit(code);
};
process.on('uncaughtException', e => { console.error(e); finish(1); });
setTimeout(() => { console.error('timed out'); finish(1); }, 120000 + 60000 * runs).unref();

let port;
for (let i = 0; i < 100 && !port; i++) {
  try { port = fs.readFileSync(path.join(profile, 'DevToolsActivePort'), 'utf8').split('\n')[0]; } catch { await sleep(100); }
}
if (!port) { console.error(`Chrome did not start (${chrome})`); finish(2); }
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const ws = new WebSocket(targets.find(t => t.type === 'page').webSocketDebuggerUrl);
await new Promise(r => ws.onopen = r);

let id = 0;
const pending = new Map();
const logs = [];
ws.onmessage = m => {
  const d = JSON.parse(m.data);
  if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); }
  if (d.method === 'Runtime.consoleAPICalled') {
    logs.push({ at: Date.now(), text: d.params.args.map(a => a.value).join(' ') });
  }
};
const send = (method, params = {}) => new Promise(r => {
  const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params }));
});
const ev = async expr => {
  const r = await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true });
  return r.result?.result?.value;
};
const waitFor = async (expr, ms) => {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) { if (await ev(expr)) return true; await sleep(200); }
  return false;
};
const shoot = async name => {
  if (!shots) return;
  fs.mkdirSync(shots, { recursive: true });
  const s = await send('Page.captureScreenshot', { format: 'png' });
  fs.writeFileSync(path.join(shots, name), Buffer.from(s.result.data, 'base64'));
};
const key = async (k, code, vk) => {
  await send('Input.dispatchKeyEvent', { type: 'keyDown', key: k, code, windowsVirtualKeyCode: vk });
  await send('Input.dispatchKeyEvent', { type: 'keyUp', key: k, code, windowsVirtualKeyCode: vk });
  await sleep(120);
};
const click = async (x, y) => {
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, button: 'none' });
  await sleep(100);
  await send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
  await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 });
};
const waitLog = async (pattern, after, ms) => {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const l = logs.find(l => l.at >= after && pattern.test(l.text));
    if (l) return l;
    await sleep(20);
  }
  return null;
};

await send('Runtime.enable');
await send('Page.enable');
await send('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: 1, mobile: false });
await send('Page.navigate', { url: `http://127.0.0.1:${httpPort}/` });
if (!await waitFor(`!document.getElementById('loading_text')`, 30000)) { console.error('the app did not start'); finish(1); }
await sleep(1500);
await ev(`(async () => {
  const b = await (await fetch('picture.png')).blob();
  const dt = new DataTransfer(); dt.items.add(new File([b], 'title.png', { type: 'image/png' }));
  const c = document.querySelector('canvas');
  for (const t of ['dragenter', 'dragover', 'drop'])
    c.dispatchEvent(new DragEvent(t, { dataTransfer: dt, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }));
  return 1; })()`);
if (!await waitFor(`document.title.startsWith('title.png')`, 30000)) { console.error('the picture did not open'); finish(1); }
await sleep(5000);
// The dock, scrolled by SCROLL points if the Dither group is below the fold (#288 put it first).
await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 1280, y: 700, button: 'none' });
await send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: 1280, y: 700, deltaX: 0, deltaY: Number(process.env.SCROLL || 0) });
await sleep(1500);
await shoot('0-opened.png');

// The Recipe tab's Method button, at 1440 × 900 (METHOD=x,y to override).
const [mx, my] = (process.env.METHOD || '1300,429').split(',').map(Number);
for (let run = 1; run <= runs; run++) {
  const load = os.loadavg().map(v => v.toFixed(1)).join(' ');
  const t0 = Date.now();
  await click(mx, my);
  const visible = await waitLog(/visible method thumbnails/, t0, 30000);
  const all = await waitLog(/rendered \d+ method thumbnails/, t0, 60000);
  await shoot(`${run}-picker.png`);
  console.log(`open ${run} (load ${load}):`);
  console.log(`  click → visible cards: ${visible ? visible.at - t0 : '—'} ms; ${visible ? visible.text : 'no log'}`);
  console.log(`  click → all cards:     ${all ? all.at - t0 : '—'} ms; ${all ? all.text : 'no log'}`);
  if (!visible || !all) failed++;
  // Close, and change the strength so the next open renders again (the cache is keyed
  // by settings).
  await key('Escape', 'Escape', 27);
  await sleep(500);
  if (run < runs) {
    const [sx, sy] = (process.env.STRENGTH || '1320,465').split(',').map(Number);
    await click(sx - 20 + 8 * run, sy);
    await sleep(3000);
  }
}
finish(failed ? 1 : 0);
