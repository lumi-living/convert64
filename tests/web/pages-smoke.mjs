// Sub-path check for the GitHub Pages build (#9, .github/workflows/pages.yml): a project
// page lives under /convert64/, so the bundle must load, start its solve worker (#265)
// and solve from there.
//
//   (cd crates/convert64-app && trunk build --release --public-url /convert64/)
//   node tests/web/pages-smoke.mjs [dist dir] [picture] [screenshot.png]
//
// Needs Node 22, python3 and Google Chrome (CHROME=path to override); no npm packages.
// It serves a temporary directory holding `convert64 -> dist` and `title.png` with
// Python's http.server (PORT, default 18731), opens /convert64/ in headless Chrome over
// the DevTools protocol, drops the picture on the page, waits for it to open, and
// prints the server's request log, the console and any exceptions. Exit code 0 when the
// app starts, the picture opens, nothing throws and every request but the favicon
// succeeds (the log shows solve-worker.js and the worker's own module fetches).
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const here = path.dirname(new URL(import.meta.url).pathname);
const repo = path.resolve(here, '..', '..');
const dist = path.resolve(process.argv[2] || path.join(repo, 'crates/convert64-app/dist'));
const picture = path.resolve(process.argv[3] || path.join(repo, 'fixtures/title/source.png'));
const shotPath = process.argv[4] ? path.resolve(process.argv[4]) : null;
if (!fs.readFileSync(path.join(dist, 'index.html'), 'utf8').includes('<base href="/convert64/"')) {
  console.error(`${dist}/index.html is not built for /convert64/: run trunk build --release --public-url /convert64/`);
  process.exit(2);
}
const sleep = ms => new Promise(r => setTimeout(r, ms));
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'c64-pages-root-'));
fs.symlinkSync(dist, path.join(root, 'convert64'));
fs.symlinkSync(picture, path.join(root, 'title.png'));
const HTTP = Number(process.env.PORT || 18731), W = 1440, H = 900;
const serverLog = [];
const server = spawn('python3', ['-m', 'http.server', String(HTTP), '--bind', '127.0.0.1', '--directory', root]);
server.stderr.on('data', d => serverLog.push(...d.toString().split('\n').filter(Boolean)));
await sleep(800);

const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'c64-pages-chrome-'));
const chrome = process.env.CHROME || (process.platform === 'darwin'
  ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
  : 'google-chrome');
const browser = spawn(chrome, ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
  '--no-first-run', '--no-default-browser-check', `--window-size=${W},${H}`, 'about:blank'], { stdio: 'ignore' });
const finish = code => {
  try { browser.kill('SIGKILL'); } catch {}
  try { server.kill(); } catch {}
  setTimeout(() => {
    try { fs.rmSync(profile, { recursive: true, force: true }); } catch {}
    try { fs.rmSync(root, { recursive: true, force: true }); } catch {}
    process.exit(code);
  }, 1000);
};
setTimeout(() => { console.error('timed out'); finish(1); }, 180000).unref();

let port;
for (let i = 0; i < 100 && !port; i++) {
  try { port = fs.readFileSync(path.join(profile, 'DevToolsActivePort'), 'utf8').split('\n')[0]; } catch { await sleep(100); }
}
const page = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find(t => t.type === 'page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise(r => ws.onopen = r);
let id = 0;
const pending = new Map();
const logs = [];
const errors = [];
ws.onmessage = m => {
  const d = JSON.parse(m.data);
  if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); }
  if (d.method === 'Runtime.consoleAPICalled') logs.push(d.params.args.map(a => a.value ?? a.description).join(' '));
  if (d.method === 'Runtime.exceptionThrown') errors.push(JSON.stringify(d.params.exceptionDetails.exception?.description ?? d.params.exceptionDetails.text));
};
const send = (method, params = {}) => new Promise(r => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const ev = async expr => (await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true })).result?.result?.value;
const waitFor = async (expr, ms) => { const t0 = Date.now(); while (Date.now() - t0 < ms) { if (await ev(expr)) return Date.now() - t0; await sleep(200); } return -1; };

await send('Runtime.enable');
await send('Page.enable');
await send('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: 1, mobile: false });
await send('Page.navigate', { url: `http://127.0.0.1:${HTTP}/convert64/` });
const started = await waitFor(`!document.getElementById('loading_text')`, 60000);
console.log(`app started: ${started >= 0 ? `yes, ${started} ms` : 'NO'}`);
await sleep(1500);
await ev(`(async () => {
  const b = await (await fetch('/title.png')).blob();
  const dt = new DataTransfer(); dt.items.add(new File([b], 'title.png', { type: 'image/png' }));
  const c = document.querySelector('canvas');
  for (const t of ['dragenter', 'dragover', 'drop'])
    c.dispatchEvent(new DragEvent(t, { dataTransfer: dt, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }));
  return 1; })()`);
const opened = await waitFor(`document.title.startsWith('title.png')`, 30000);
console.log(`picture opened: ${opened >= 0 ? 'yes' : 'NO'} (title "${await ev('document.title')}")`);
await sleep(8000);
if (shotPath) {
  const s = await send('Page.captureScreenshot', { format: 'png' });
  fs.writeFileSync(shotPath, Buffer.from(s.result.data, 'base64'));
}
console.log('--- server log');
for (const l of serverLog) console.log(l.replace(/^.*?"/, '"'));
console.log('--- console');
for (const l of logs) console.log(l);
console.log('--- exceptions');
for (const e of errors) console.log(e);
const bad = serverLog.filter(l => / (4|5)\d\d /.test(l) && !l.includes('favicon'));
console.log(bad.length ? `FAIL: ${bad.length} failed requests` : 'ok: every request (bar favicon) succeeded');
finish(started >= 0 && opened >= 0 && !bad.length && !errors.length ? 0 : 1);
