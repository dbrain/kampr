// Does a real ctrl+V reach the page when the focus is not on an editable element?
//
// Synthetic ClipboardEvents already answer the seam (#365); they cannot answer this, because a
// synthetic event is dispatched at a target this code chose. Only the browser decides where a real
// paste shortcut goes, so this drives a real key chord against a real system clipboard through CDP.
const http = require('http');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const os = require('os');

// `ws` is not a dependency of anything here; it is whatever the Kotlin/JS toolchain has already
// unpacked. Nothing in this repository has a package.json to add it to, and a probe that needs an
// npm install is a probe nobody re-runs.
const WS_PATH = ['client/build/wasm/node_modules/ws', 'client/build/js/node_modules/ws']
  .map((rel) => path.join(__dirname, '../../..', rel))
  .find((candidate) => fs.existsSync(candidate));
if (!WS_PATH) {
  console.error("no `ws` on disk — run `./gradlew :shared:wasmJsBrowserTest` from client/ once, which unpacks it");
  process.exit(1);
}
const WebSocket = require(WS_PATH);

const PORT = 9333;
const PAGE = 'file://' + path.join(__dirname, 'page.html');

const get = (url) => new Promise((resolve, reject) => {
  http.get(url, (res) => { let b = ''; res.on('data', (c) => b += c); res.on('end', () => resolve(JSON.parse(b))); })
    .on('error', reject);
});
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

class Cdp {
  constructor(ws) { this.ws = ws; this.next = 1; this.waiting = new Map(); 
    ws.on('message', (raw) => {
      const msg = JSON.parse(raw);
      if (msg.id && this.waiting.has(msg.id)) {
        const { resolve, reject } = this.waiting.get(msg.id);
        this.waiting.delete(msg.id);
        msg.error ? reject(new Error(JSON.stringify(msg.error))) : resolve(msg.result);
      }
    });
  }
  send(method, params = {}, sessionId) {
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.waiting.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params, sessionId }));
    });
  }
}

async function evaluate(cdp, expression) {
  const r = await cdp.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
  return r.result.value;
}

// A real chord, not a synthetic event: the browser process reads the system clipboard and decides
// which element the paste is delivered to, which is the entire question.
async function ctrlV(cdp) {
  const base = { modifiers: 2, windowsVirtualKeyCode: 86, nativeVirtualKeyCode: 86, key: 'v', code: 'KeyV' };
  await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', ...base });
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', ...base });
  await sleep(300);
}

(async () => {
  const chrome = spawn(process.env.CHROMIUM || 'chromium', [
    '--headless=new', `--remote-debugging-port=${PORT}`, '--no-sandbox',
    `--user-data-dir=${fs.mkdtempSync(path.join(os.tmpdir(), 'kampr-paste-probe-'))}`,
    '--allow-file-access-from-files', 'about:blank',
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

  await cdp.send('Browser.grantPermissions', { permissions: ['clipboardReadWrite', 'clipboardSanitizedWrite'] });
  await cdp.send('Page.enable');
  // A headless page is not "focused" and the async clipboard API refuses an unfocused document.
  // Focus emulation is what makes the browser treat it as the foreground tab it would be.
  await cdp.send('Emulation.setFocusEmulationEnabled', { enabled: true });
  await cdp.send('Page.bringToFront');
  await cdp.send('Runtime.enable');
  await cdp.send('Page.navigate', { url: PAGE });
  await sleep(1200);

  const version = await get(`http://127.0.0.1:${PORT}/json/version`);

  // A one-pixel PNG on the real system clipboard, written by the page itself so it is a genuine
  // image/png clipboard entry rather than a string that looks like one.
  // Two payloads, because the browser decides *where* a paste goes before it looks at what is on
  // the clipboard: text is what a headless Chromium will always accept, and an image is the case
  // that matters. Both are recorded rather than assumed.
  const seeded = await evaluate(cdp, `
    (async () => {
      const out = {};
      const b64 = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR4nGMAAQAABQABDQottAAAAABJRU5ErkJggg==';
      try { await navigator.clipboard.writeText('probe-paste-text'); out.text = 'ok'; }
      catch (e) { out.text = e.name + ': ' + e.message; }
      try {
        const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
        await navigator.clipboard.write([new ClipboardItem({ 'image/png': new Blob([bytes], { type: 'image/png' }) })]);
        out.image = 'ok';
      } catch (e) { out.image = e.name + ': ' + e.message; }
      // The other way a picture gets on a clipboard: the browser's own copy command over a
      // selection that contains one, which uses Chromium's encoder rather than the async API's
      // sanitising decode.
      try {
        const img = document.createElement('img');
        img.src = 'data:image/png;base64,' + b64;
        document.body.appendChild(img);
        await img.decode();
        const range = document.createRange();
        range.selectNode(img);
        const sel = getSelection();
        sel.removeAllRanges();
        sel.addRange(range);
        out.copyCommand = document.execCommand('copy') ? 'ok' : 'refused';
        sel.removeAllRanges();
      } catch (e) { out.copyCommand = e.name + ': ' + e.message; }
      return out;
    })()
  `);

  const readings = {};
  for (const where of ['editable', 'bare', 'shadow']) {
    const active = await evaluate(cdp, `window.__focus(${JSON.stringify(where)})`);
    await ctrlV(cdp);
    readings[where] = { activeElement: active, seen: await evaluate(cdp, 'window.__seen') };
  }

  console.log(JSON.stringify({ browser: version.Browser, seeded, readings }, null, 2));
  ws.close();
  chrome.kill();
})().catch((e) => { console.error('probe failed:', e.message); process.exit(1); });
