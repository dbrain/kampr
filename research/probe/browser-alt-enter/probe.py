# What does a real Chrome deliver for alt+enter to the offscreen input the wasm client installs?
#
# Driven through CDP's Input.dispatchKeyEvent, which goes in at the browser's own input pipeline
# rather than as a synthesised DOM event, against the handler copied verbatim out of
# PaneTextInput.wasmJs.kt. The question is whether the first chord after a focus change carries
# its modifiers, and whether it reaches the element that holds the handler at all.
import json, os, subprocess, sys, tempfile, time, urllib.request, http.server, threading, functools

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from ws import WS

HERE = os.path.dirname(os.path.abspath(__file__))
PORT = 8823
CDP_PORT = 9335


class Cdp:
    def __init__(self, url):
        rest = url.split('://', 1)[1]
        hostport, path = rest.split('/', 1)
        host, port = hostport.split(':')
        self.ws = WS(host, int(port), '/' + path)
        self.id = 0

    def send(self, method, **params):
        self.id += 1
        self.ws.send(json.dumps({'id': self.id, 'method': method, 'params': params}))
        while True:
            op, payload = self.ws.frame()
            if op not in (0x1, 0x2):
                continue
            msg = json.loads(payload)
            if msg.get('id') == self.id:
                if 'error' in msg:
                    raise RuntimeError(msg['error'])
                return msg.get('result', {})

    def eval(self, expr):
        r = self.send('Runtime.evaluate', expression=expr, returnByValue=True, awaitPromise=True)
        if 'exceptionDetails' in r:
            raise RuntimeError(json.dumps(r['exceptionDetails']))
        return r['result'].get('value')


ALT = 1
ENTER_VK = 13


def key(cdp, kind, text_key, code, vk, modifiers):
    p = dict(type=kind, key=text_key, code=code, windowsVirtualKeyCode=vk,
             nativeVirtualKeyCode=vk, modifiers=modifiers)
    cdp.send('Input.dispatchKeyEvent', **p)


def alt_enter(cdp):
    key(cdp, 'rawKeyDown', 'Alt', 'AltLeft', 18, ALT)
    key(cdp, 'rawKeyDown', 'Enter', 'Enter', ENTER_VK, ALT)
    key(cdp, 'keyUp', 'Enter', 'Enter', ENTER_VK, ALT)
    key(cdp, 'keyUp', 'Alt', 'AltLeft', 18, 0)
    time.sleep(0.05)


def plain_enter(cdp):
    key(cdp, 'rawKeyDown', 'Enter', 'Enter', ENTER_VK, 0)
    key(cdp, 'keyUp', 'Enter', 'Enter', ENTER_VK, 0)
    time.sleep(0.05)


def reading(cdp):
    return {'delivered': cdp.eval('JSON.stringify(window.__delivered)'),
            'log': json.loads(cdp.eval('JSON.stringify(window.__log)'))}


def main():
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=HERE)
    srv = http.server.ThreadingHTTPServer(('127.0.0.1', PORT), handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    profile = tempfile.mkdtemp()
    chrome = subprocess.Popen([
        os.environ.get('CHROME', '/usr/bin/google-chrome-stable'), '--headless=new', f'--remote-debugging-port={CDP_PORT}',
        f'--user-data-dir={profile}', '--no-first-run', '--no-default-browser-check',
        '--disable-gpu', f'http://127.0.0.1:{PORT}/page.html',
    ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        target = None
        for _ in range(80):
            try:
                tabs = json.load(urllib.request.urlopen(f'http://127.0.0.1:{CDP_PORT}/json'))
                target = next((t for t in tabs if t['type'] == 'page' and 'page.html' in t['url']), None)
                if target:
                    break
            except Exception:
                pass
            time.sleep(0.25)
        if not target:
            raise SystemExit('chrome never came up')
        cdp = Cdp(target['webSocketDebuggerUrl'])
        cdp.send('Runtime.enable')
        cdp.send('Page.enable')
        cdp.send('Input.setIgnoreInputEvents', ignore=False)
        out = {}

        # 1. The very first chord on a freshly loaded page, with the input focused as the pane
        #    focuses it.
        cdp.eval("window.__reset(); window.__kamprInput.hold = true; window.__kamprInput.el.focus()")
        alt_enter(cdp)
        out['firstChordAfterFocus'] = reading(cdp)

        # 2. A second one, so a first-versus-later difference would show.
        cdp.eval('window.__reset()')
        alt_enter(cdp)
        out['secondChord'] = reading(cdp)

        # 3. Plain enter, for the shape of the working case.
        cdp.eval('window.__reset()')
        plain_enter(cdp)
        out['plainEnter'] = reading(cdp)

        # 4. The first chord after the focus went elsewhere and the per-frame reclaim took it back.
        cdp.eval("window.__reset(); document.getElementById('canvas').focus()")
        alt_enter(cdp)
        out['chordWhileTheCanvasHoldsTheFocus'] = reading(cdp)

        cdp.eval("window.__reset(); window.__kamprInput.el.focus()")
        alt_enter(cdp)
        out['firstChordAfterTheReclaim'] = reading(cdp)

        # 5. The rival text field, which is what the reclaim stands down for.
        cdp.eval("window.__reset(); document.getElementById('rival').focus()")
        alt_enter(cdp)
        out['chordWhileATextFieldHoldsTheFocus'] = reading(cdp)

        # 6. Alt pressed and released on its own first, which is what focuses a browser menu.
        cdp.eval("window.__reset(); window.__kamprInput.el.focus()")
        key(cdp, 'rawKeyDown', 'Alt', 'AltLeft', 18, ALT)
        key(cdp, 'keyUp', 'Alt', 'AltLeft', 18, 0)
        time.sleep(0.1)
        alt_enter(cdp)
        out['chordAfterALoneAltPress'] = reading(cdp)

        print(json.dumps(out, indent=1))
    finally:
        chrome.terminate()
        try:
            chrome.wait(timeout=10)
        except subprocess.TimeoutExpired:
            chrome.kill()
        srv.shutdown()


main()
