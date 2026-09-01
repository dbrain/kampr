// Does a mouse drag select text in the real web build, or does it take a long press?
//
// The report was "selecting text on wasm desktop needs a long press", and the transcript is
// ordinary Compose `SelectionContainer` text. Whether that container takes a drag turns on the
// pointer type the browser hands it, so this drives a real drag over a real page and counts the
// selection wash in a screenshot — the same instrument `SelectionTest` uses on the JVM.
const http = require('http');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const os = require('os');

const WS_PATH = ['client/build/wasm/node_modules/ws', 'client/build/js/node_modules/ws']
  .map((rel) => path.join(__dirname, '../../..', rel))
  .find((candidate) => fs.existsSync(candidate));
const WebSocket = require(WS_PATH);

const PORT = 9335;
const PAGE = process.env.PAGE || 'http://127.0.0.1:8811/?probe=select';
const OUT = process.env.OUT || '/tmp/kampr-select-probe';

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

async function shot(cdp, name) {
  const r = await cdp.send('Page.captureScreenshot', { format: 'png' });
  const file = path.join(OUT, name + '.png');
  fs.writeFileSync(file, Buffer.from(r.data, 'base64'));
  return file;
}

const MOUSE = { pointerType: 'mouse', button: 'left', buttons: 1, clickCount: 1 };

async function dragQuickly(cdp, y) {
  await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: 40, y, ...MOUSE });
  for (const x of [120, 260, 420, 560]) {
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, ...MOUSE });
    await sleep(25);
  }
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 560, y, ...MOUSE, buttons: 0 });
  await sleep(250);
}

async function dragAfterHolding(cdp, y) {
  await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: 40, y, ...MOUSE });
  await sleep(900);
  for (const x of [120, 260, 420, 560]) {
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, ...MOUSE });
    await sleep(25);
  }
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 560, y, ...MOUSE, buttons: 0 });
  await sleep(250);
}

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const chrome = spawn(process.env.CHROMIUM || 'chromium', [
    '--headless=new', `--remote-debugging-port=${PORT}`, '--no-sandbox',
    `--user-data-dir=${fs.mkdtempSync(path.join(os.tmpdir(), 'kampr-select-probe-'))}`,
    '--window-size=900,600', '--hide-scrollbars', 'about:blank',
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
  await sleep(4500);

  const files = { before: await shot(cdp, 'before') };
  // The text sits a little below the top padding; find the row the glyphs are actually on by
  // sweeping, rather than assuming a layout.
  const y = Number(process.env.Y || 40);
  await dragQuickly(cdp, y);
  files.afterDrag = await shot(cdp, 'after-drag');
  // Click away, then the gesture the report says is needed.
  await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: 800, y: 500, ...MOUSE });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 800, y: 500, ...MOUSE, buttons: 0 });
  await sleep(200);
  await dragAfterHolding(cdp, y);
  files.afterHeldDrag = await shot(cdp, 'after-held-drag');

  const version = await get(`http://127.0.0.1:${PORT}/json/version`);
  console.log(JSON.stringify({ browser: version.Browser, page: PAGE, y, files }, null, 2));
  ws.close();
  chrome.kill();
})().catch((e) => { console.error('probe failed:', e.message); process.exit(1); });
