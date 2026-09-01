// What kind of pointer does Compose see when a real mouse drags across a real browser?
//
// The whole desk-side selection story turns on it: `terminalGestures` gives the drag to the
// selection when `down.type == PointerType.Mouse`, and `SelectionContainer` picks its mouse
// gesture set off the same reading. If CMP's web backend reported anything else, both would be
// dead code on the one platform the report came from — the #233 shape.
//
// Driven against the real web build (`?probe=pointer`, a throwaway branch in webApp's Main.kt)
// through CDP, because only the browser decides what a `pointerdown` looks like.
const http = require('http');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const os = require('os');

const WS_PATH = ['client/build/wasm/node_modules/ws', 'client/build/js/node_modules/ws']
  .map((rel) => path.join(__dirname, '../../..', rel))
  .find((candidate) => fs.existsSync(candidate));
if (!WS_PATH) {
  console.error("no `ws` on disk — run `./gradlew :shared:wasmJsBrowserTest` from client/ once, which unpacks it");
  process.exit(1);
}
const WebSocket = require(WS_PATH);

const PORT = 9334;
const PAGE = process.env.PAGE || 'http://127.0.0.1:8811/?probe=pointer';

const get = (url) => new Promise((resolve, reject) => {
  http.get(url, (res) => { let b = ''; res.on('data', (c) => b += c); res.on('end', () => resolve(JSON.parse(b))); })
    .on('error', reject);
});
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

class Cdp {
  constructor(ws) {
    this.ws = ws; this.next = 1; this.waiting = new Map();
    ws.on('message', (raw) => {
      const msg = JSON.parse(raw);
      if (msg.id && this.waiting.has(msg.id)) {
        const { resolve, reject } = this.waiting.get(msg.id);
        this.waiting.delete(msg.id);
        msg.error ? reject(new Error(JSON.stringify(msg.error))) : resolve(msg.result);
      }
    });
  }
  send(method, params = {}) {
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.waiting.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }
}

async function evaluate(cdp, expression) {
  const r = await cdp.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
  return r.result.value;
}

async function drag(cdp, kind) {
  const common = kind === 'touch'
    ? { pointerType: 'touch', button: 'left', buttons: 1, force: 1 }
    : { pointerType: 'mouse', button: 'left', buttons: 1, clickCount: 1 };
  await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: 120, y: 200, ...common });
  for (const x of [180, 260, 340]) {
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y: 200, ...common });
    await sleep(30);
  }
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 340, y: 200, ...common, buttons: 0 });
  await sleep(200);
}

(async () => {
  const chrome = spawn(process.env.CHROMIUM || 'chromium', [
    '--headless=new', `--remote-debugging-port=${PORT}`, '--no-sandbox',
    `--user-data-dir=${fs.mkdtempSync(path.join(os.tmpdir(), 'kampr-pointer-probe-'))}`,
    '--window-size=900,700', 'about:blank',
  ], { stdio: 'ignore' });

  let target;
  for (let i = 0; i < 60; i++) {
    try { const list = await get(`http://127.0.0.1:${PORT}/json/list`); target = list.find((t) => t.type === 'page'); if (target) break; } catch (_) {}
    await sleep(250);
  }
  if (!target) { chrome.kill(); throw new Error('chromium never came up'); }

  const ws = new WebSocket(target.webSocketDebuggerUrl, { perMessageDeflate: false });
  await new Promise((r) => ws.on('open', r));
  const cdp = new Cdp(ws);

  await cdp.send('Page.enable');
  await cdp.send('Runtime.enable');
  await cdp.send('Emulation.setFocusEmulationEnabled', { enabled: true });
  await cdp.send('Page.bringToFront');
  await cdp.send('Page.navigate', { url: PAGE });
  await sleep(4000);

  const painted = await evaluate(cdp, `!!document.querySelector('canvas') || !!(document.body.shadowRoot) || document.body.innerHTML.length`);
  const readings = {};
  for (const kind of ['mouse', 'touch']) {
    await evaluate(cdp, 'globalThis.__kamprPointerProbe = []');
    await drag(cdp, kind);
    readings[kind] = await evaluate(cdp, 'globalThis.__kamprPointerProbe || null');
  }

  const version = await get(`http://127.0.0.1:${PORT}/json/version`);
  console.log(JSON.stringify({ browser: version.Browser, page: PAGE, painted, readings }, null, 2));
  ws.close();
  chrome.kill();
})().catch((e) => { console.error('probe failed:', e.message); process.exit(1); });
