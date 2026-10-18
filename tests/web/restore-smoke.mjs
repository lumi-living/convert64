// Browser smoke test for autosave, restore and Save (#241, usability finding C3).
//
//   (cd crates/convert64-app && trunk build --release)
//   node tests/web/restore-smoke.mjs [dist dir] [picture]
//
// Needs only Node 22 (global fetch and WebSocket) and Google Chrome (CHROME=path to
// override where it is looked for); no npm packages. It serves the bundle and the
// picture on a free port, starts headless Chrome with a throwaway profile, drives it
// over the DevTools protocol and stops it at the end:
//
//   1. drops the picture on the page and waits for the result;
//   2. nudges the layer 3 px right (`V`, then three right-arrow presses);
//   3. waits for the autosave and reads the record from the page's storage (OPFS);
//   4. reloads (checking that the unsaved-changes guard asks first), waits for the
//      restore bar and clicks Restore;
//   5. checks the restored project: a no-op edit (left, right) makes the autosave write
//      the restored session again, and its layer is still at x = 3; the tab title has
//      the unsaved marker;
//   6. presses Cmd/Ctrl S with `showSaveFilePicker` hidden (as in Firefox and Safari)
//      and checks that a real download completed (`Browser.downloadProgress`) and holds
//      the nudged project.
//
// Exit code 0 when every check passes. `SHOTS=dir` also saves screenshots there.

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

// The bundle and the picture, as `/title.png`.
const types = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.png': 'image/png' };
const server = http.createServer((req, res) => {
  const url = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  const file = url === '/title.png' ? picture : path.join(dist, url === '/' ? 'index.html' : url);
  if (!file.startsWith(dist) && file !== picture) { res.writeHead(403); return res.end(); }
  fs.readFile(file, (err, data) => {
    if (err) { res.writeHead(404); return res.end(); }
    res.writeHead(200, { 'content-type': types[path.extname(file)] || 'application/octet-stream' });
    res.end(data);
  });
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const httpPort = server.address().port;

const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'convert64-smoke-'));
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

// Chrome writes its port to the profile.
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
const logs = [];
const downloadsDone = [];
const dialogs = [];
const onMessage = m => {
  const d = JSON.parse(m.data);
  // The unsaved-changes guard: "Leave site?" on the reload. Leave.
  if (d.method === 'Page.javascriptDialogOpening') {
    dialogs.push(d.params.type);
    ws.send(JSON.stringify({ id: ++id, method: 'Page.handleJavaScriptDialog', params: { accept: true } }));
  }
  if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); }
  if (d.method === 'Runtime.consoleAPICalled') logs.push(d.params.args.map(a => a.value).join(' '));
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
const until = async (what, expr, ms = 30000) => {
  const ok = await waitFor(expr, ms);
  if (!ok) check(false, `${what} (timed out after ${ms / 1000} s)`);
  return ok;
};
const shoot = async name => {
  if (!shots) return;
  fs.mkdirSync(shots, { recursive: true });
  const s = await send('Page.captureScreenshot', { format: 'png' });
  fs.writeFileSync(path.join(shots, name), Buffer.from(s.result.data, 'base64'));
};
const command = process.platform === 'darwin' ? 4 : 2; // egui's Cmd: Meta on macOS, else Ctrl
const key = async (k, code, vk, modifiers = 0) => {
  const text = k.length === 1 && !modifiers ? k : undefined;
  await send('Input.dispatchKeyEvent', { type: 'keyDown', key: k, code, windowsVirtualKeyCode: vk, modifiers, text });
  await send('Input.dispatchKeyEvent', { type: 'keyUp', key: k, code, windowsVirtualKeyCode: vk, modifiers });
  await sleep(120);
};
const click = async (x, y) => {
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, button: 'none' });
  await sleep(100);
  await send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
  await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 });
};
// The newest autosaved record in OPFS, parsed, with its project parsed too.
const record = () => ev(`(async () => {
  const root = await navigator.storage.getDirectory();
  let dir; try { dir = await root.getDirectoryHandle('convert64-autosave'); } catch { return null; }
  let best = null;
  for await (const [name, h] of dir.entries()) {
    if (!name.startsWith('session-')) continue;
    const r = JSON.parse(await (await h.getFile()).text());
    if (!best || r.modified_ms > best.modified_ms) best = r;
  }
  if (best) best.project = JSON.parse(best.project);
  return best;
})()`);
const loaded = () => until('the app starts', `!document.getElementById('loading_text')`);

await send('Runtime.enable');
await send('Page.enable');
await send('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: 1, mobile: false });
await sendOn(browserWs, 'Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: downloads, eventsEnabled: true });
const origin = `http://127.0.0.1:${httpPort}/`;
await send('Page.navigate', { url: origin });
await loaded();
await sleep(1500);

// 1. The picture, dropped on the page.
await ev(`(async () => {
  const b = await (await fetch('title.png')).blob();
  const dt = new DataTransfer(); dt.items.add(new File([b], 'title.png', { type: 'image/png' }));
  const c = document.querySelector('canvas');
  for (const t of ['dragenter', 'dragover', 'drop'])
    c.dispatchEvent(new DragEvent(t, { dataTransfer: dt, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }));
  return 1; })()`);
await until('the picture opens', `document.title.startsWith('title.png')`);
await sleep(4000);

// 2. Nudge 3 px right with the arrow keys (the Nudge tool).
await click(500, 400);
await key('v', 'KeyV', 86);
for (let i = 0; i < 3; i++) await key('ArrowRight', 'ArrowRight', 39);
await until('the title marks unsaved changes', `document.title.startsWith('title.png*')`);

// 3. The autosave, about a second later.
await until('the autosave writes the session', `(async () => {
  const root = await navigator.storage.getDirectory();
  try { const d = await root.getDirectoryHandle('convert64-autosave');
    for await (const n of d.keys()) if (n.startsWith('session-')) return true; } catch {}
  return false; })()`, 10000);
await sleep(500);
const before = await record();
check(before && before.layers === 1 && before.project_key !== before.saved_key, 'the record holds unsaved work, one layer');
check(before && before.sources.length === 1 && !JSON.stringify(before.project).includes('png_base64'),
  'the source is a blob by content, not in the JSON');
console.log(`     autosave in ${logs.find(l => l.includes('autosave in')) || '?'}; layer 0 transform: ${JSON.stringify(before?.project?.layers?.[0]?.transform)}`);

// 4. Reload (the guard asks first: unsaved changes), then the restore bar; Restore.
await send('Page.reload');
await sleep(500);
await loaded();
check(dialogs.includes('beforeunload'), 'reloading with unsaved changes asks first (beforeunload)');
await sleep(2500);
await shoot('browser-restore-bar.png');
const bar = await ev(`document.title`);
check(bar === 'Convert64', `after the reload nothing is open (title "${bar}")`);
// The bar's Restore button follows its text, whose width is fixed for this picture
// ("title.png, 1 layer, just now"); RESTORE_X / RESTORE_Y override where to click.
const restoreX = Number(process.env.RESTORE_X || 0);
const barY = Number(process.env.RESTORE_Y || 0);
// Left to right along the bar: the text ignores clicks, and Restore comes before Discard.
const candidates = restoreX
  ? [[restoreX, barY]]
  : Array.from({ length: 16 }, (_, i) => [280 + i * 10, barY || 62]);
let restored = false;
for (const [x, y] of candidates) {
  await click(x, y);
  if (await waitFor(`document.title.startsWith('title.png*')`, 600)) {
    restored = true;
    console.log(`     Restore clicked at ${x},${y}`);
    break;
  }
}
check(restored, 'Restore opens the session as unsaved work (title has the marker)');
await sleep(4000);
await shoot('browser-restored.png');

// 5. The nudge survived: a no-op edit makes the autosave write the restored session.
await click(500, 400);
await key('ArrowLeft', 'ArrowLeft', 37);
await key('ArrowRight', 'ArrowRight', 39);
await sleep(2500);
const after = await record();
check(after && before && after.id === before.id, 'the restored session goes on in the same record');
check(after && before && JSON.stringify(after.project.layers[0].transform) === JSON.stringify(before.project.layers[0].transform),
  `the nudge survived the reload (${JSON.stringify(after?.project?.layers?.[0]?.transform)})`);

// 6. Save: a real download, no overlay (showSaveFilePicker hidden, as in Firefox/Safari).
await ev(`(() => { window.showSaveFilePicker = undefined; return 1; })()`);
await key('s', 'KeyS', 83, command);
const t0 = Date.now();
while (!downloadsDone.length && Date.now() - t0 < 10000) await sleep(200);
check(downloadsDone.length === 1, 'Save downloads one file');
await sleep(300);
await shoot('browser-saved.png');
const files = fs.readdirSync(downloads);
const saved = files.find(f => f.endsWith('.convert64'));
check(saved === 'title.convert64', `the download is title.convert64 (${files.join(', ')})`);
if (saved) {
  const project = JSON.parse(fs.readFileSync(path.join(downloads, saved), 'utf8'));
  check(JSON.stringify(project.layers[0].transform) === JSON.stringify(before?.project?.layers?.[0]?.transform),
    'the downloaded project has the nudge');
  check(JSON.stringify(project).includes('png_base64'), 'the downloaded project embeds its source');
  console.log(`     downloaded ${saved}: ${fs.statSync(path.join(downloads, saved)).size} B`);
}
check(await waitFor(`document.title === 'title.png — Convert64'`, 5000),
  'after the download the title has no marker');

ws.close();
browserWs.close();
console.log(failed ? `${failed} check(s) failed` : 'all checks passed');
finish(failed ? 1 : 0);
