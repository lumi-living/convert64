// Browser smoke test for the export set (#287, usability review M4): Export Again
// (Shift Cmd/Ctrl E) writes every row of the project's export set through #241's direct
// save path.
//
//   (cd crates/convert64-app && trunk build --release)
//   node tests/web/export-smoke.mjs [dist dir] [picture]
//
// Needs only Node 22 and Google Chrome (CHROME=path to override), no npm packages, as
// restore-smoke.mjs. It serves the bundle, starts headless Chrome with a throwaway
// profile and drives it over the DevTools protocol:
//
//   1. drops a project whose export set is Koala, PRG and a 1× PNG on the page;
//   2. with `showDirectoryPicker` answering with a folder in the page's OPFS (headless
//      Chrome shows no picker), presses Shift Cmd E and checks the three files and
//      their sizes in that folder;
//   3. presses it again: the same folder is written without asking again;
//   4. reloads and, with `showDirectoryPicker` hidden (as in Firefox and Safari), presses
//      it once more and checks one real download per file (`Browser.downloadProgress`).
//
// Exit code 0 when every check passes. `SHOTS=dir` also saves a screenshot there.

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
const chrome = process.env.CHROME || (process.platform === 'darwin'
  ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
  : 'google-chrome');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const W = 1440, H = 900;

if (!fs.existsSync(path.join(dist, 'index.html'))) {
  console.error(`no ${dist}/index.html: run (cd crates/convert64-app && trunk build --release)`);
  process.exit(2);
}

// The project: the picture embedded, and the export set.
const project = JSON.stringify({
  schema_version: 3,
  target: 'multicolor-bitmap',
  layers: [{ name: 'title', source: { png_base64: fs.readFileSync(picture).toString('base64') } }],
  exports: [{ format: 'picture' }, { format: 'prg' }, { format: 'png', zoom: 1 }],
});

const types = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm' };
const server = http.createServer((req, res) => {
  const url = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  if (url === '/title.convert64') {
    res.writeHead(200, { 'content-type': 'application/json' });
    return res.end(project);
  }
  const file = path.join(dist, url === '/' ? 'index.html' : url);
  if (!file.startsWith(dist)) { res.writeHead(403); return res.end(); }
  fs.readFile(file, (err, data) => {
    if (err) { res.writeHead(404); return res.end(); }
    res.writeHead(200, { 'content-type': types[path.extname(file)] || 'application/octet-stream' });
    res.end(data);
  });
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const httpPort = server.address().port;

const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'convert64-export-smoke-'));
const downloads = path.join(profile, 'downloads');
fs.mkdirSync(downloads);
const browser = spawn(chrome, [
  '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
  '--no-first-run', '--no-default-browser-check', `--window-size=${W},${H}`, 'about:blank',
], { stdio: 'ignore' });

let failed = 0;
const check = (ok, what) => { console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); if (!ok) failed++; };
const finish = async code => {
  const exited = new Promise(r => browser.once('exit', r));
  try { browser.kill('SIGKILL'); } catch {}
  await Promise.race([exited, sleep(5000)]);
  server.close();
  try { fs.rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 }); } catch {}
  process.exit(code);
};
process.on('uncaughtException', e => { console.error(e); finish(1); });
setTimeout(() => { console.error('timed out'); finish(1); }, 180000).unref();

let port;
for (let i = 0; i < 100 && !port; i++) {
  try { port = fs.readFileSync(path.join(profile, 'DevToolsActivePort'), 'utf8').split('\n')[0]; } catch { await sleep(100); }
}
if (!port) { console.error(`Chrome did not start (${chrome})`); finish(2); }
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find(t => t.type === 'page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise(r => ws.onopen = r);
const browserWs = new WebSocket((await (await fetch(`http://127.0.0.1:${port}/json/version`)).json()).webSocketDebuggerUrl);
await new Promise(r => browserWs.onopen = r);

let id = 0;
const pending = new Map();
const downloadsDone = [];
const onMessage = m => {
  const d = JSON.parse(m.data);
  if (d.method === 'Page.javascriptDialogOpening') {
    ws.send(JSON.stringify({ id: ++id, method: 'Page.handleJavaScriptDialog', params: { accept: true } }));
  }
  if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); }
  if (d.method === 'Browser.downloadProgress' && d.params.state === 'completed') downloadsDone.push(d.params);
};
ws.onmessage = onMessage;
browserWs.onmessage = onMessage;
const sendOn = (sock, method, params = {}) => new Promise(r => {
  const i = ++id; pending.set(i, r); sock.send(JSON.stringify({ id: i, method, params }));
});
const send = (method, params) => sendOn(ws, method, params);
const ev = async expr => {
  const r = await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) throw new Error(`${expr}: ${JSON.stringify(r.result.exceptionDetails)}`);
  return r.result?.result?.value;
};
const waitFor = async (expr, ms) => {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) { if (await ev(expr)) return true; await sleep(200); }
  return false;
};
const command = process.platform === 'darwin' ? 4 : 2; // egui's Cmd: Meta on macOS, else Ctrl
const shift = 8;
const exportAgain = async () => {
  const modifiers = command | shift;
  await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'E', code: 'KeyE', windowsVirtualKeyCode: 69, modifiers });
  await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'E', code: 'KeyE', windowsVirtualKeyCode: 69, modifiers });
};
const click = async (x, y) => {
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, button: 'none' });
  await send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
  await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 });
};
// The files in the OPFS folder the stubbed picker hands out, with their sizes.
const folder = () => ev(`(async () => {
  const root = await navigator.storage.getDirectory();
  let dir; try { dir = await root.getDirectoryHandle('export'); } catch { return {}; }
  const out = {};
  for await (const [name, h] of dir.entries()) out[name] = (await h.getFile()).size;
  return out;
})()`);
// The project dropped on the page, then the full solve.
const open = async () => {
  await waitFor(`!document.getElementById('loading_text')`, 30000);
  await sleep(1500);
  await ev(`(async () => {
    const b = await (await fetch('title.convert64')).blob();
    const dt = new DataTransfer(); dt.items.add(new File([b], 'title.convert64'));
    const c = document.querySelector('canvas');
    for (const t of ['dragenter', 'dragover', 'drop'])
      c.dispatchEvent(new DragEvent(t, { dataTransfer: dt, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }));
    return 1; })()`);
  check(await waitFor(`document.title.startsWith('title')`, 30000), 'the project opens');
  await sleep(5000);
  await click(500, 400);
};

await send('Runtime.enable');
await send('Page.enable');
await send('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: 1, mobile: false });
await sendOn(browserWs, 'Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: downloads, eventsEnabled: true });
// The folder picker answers with a folder in OPFS and counts how often it was asked.
await send('Page.addScriptToEvaluateOnNewDocument', {
  source: `window.__pickerCalls = 0;
    window.showDirectoryPicker = async () => {
      window.__pickerCalls++;
      return (await navigator.storage.getDirectory()).getDirectoryHandle('export', { create: true });
    };`,
});
await send('Page.navigate', { url: `http://127.0.0.1:${httpPort}/` });
await open();

// 2. Export Again with no folder yet: the picker, then every row's file.
const t0 = Date.now();
await exportAgain();
const want = ['title.kla', 'title.prg', 'title.png'];
let files = {};
for (let i = 0; i < 50 && Object.keys(files).length < 3; i++) { files = await folder(); await sleep(200); }
console.log(`     wrote ${JSON.stringify(files)} in ${Date.now() - t0} ms`);
check(want.every(n => n in files) && Object.keys(files).length === 3,
  'Export Again writes the three rows into the folder picked');
check(files['title.kla'] === 10003, 'the Koala file is 10 003 bytes');
check(files['title.png'] > 0 && files['title.prg'] > 0, 'the PNG and the PRG are written');
check(await ev('window.__pickerCalls') === 1, 'the folder was asked for once');
if (shots) {
  fs.mkdirSync(shots, { recursive: true });
  const s = await send('Page.captureScreenshot', { format: 'png' });
  fs.writeFileSync(path.join(shots, 'browser-exported.png'), Buffer.from(s.result.data, 'base64'));
}

// 3. Again: the same folder, not asked again.
await ev(`(async () => { const d = await (await navigator.storage.getDirectory()).getDirectoryHandle('export');
  await d.removeEntry('title.kla'); return 1; })()`);
await exportAgain();
let again = {};
for (let i = 0; i < 50 && !('title.kla' in again); i++) { again = await folder(); await sleep(200); }
check('title.kla' in again, 'Export Again writes into the same folder again');
check(await ev('window.__pickerCalls') === 1, 'without asking for the folder again');

// 4. A browser without the folder picker: one download per file.
await send('Page.addScriptToEvaluateOnNewDocument', { source: 'window.showDirectoryPicker = undefined;' });
await send('Page.reload');
await sleep(500);
await open();
await exportAgain();
const t1 = Date.now();
while (downloadsDone.length < 3 && Date.now() - t1 < 15000) await sleep(200);
await sleep(300);
const got = fs.readdirSync(downloads).sort();
check(JSON.stringify(got) === JSON.stringify(['title.kla', 'title.png', 'title.prg']),
  `without a folder picker each file is a download (${got.join(', ')})`);
if (got.includes('title.kla')) {
  check(fs.statSync(path.join(downloads, 'title.kla')).size === 10003, 'the downloaded Koala file is 10 003 bytes');
}

ws.close();
browserWs.close();
console.log(failed ? `${failed} check(s) failed` : 'all checks passed');
finish(failed ? 1 : 0);
