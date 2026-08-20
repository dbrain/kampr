package dev.kampr.terminal.spike

import kotlin.js.ExperimentalWasmJsInterop
import kotlinx.browser.window

@OptIn(ExperimentalWasmJsInterop::class)
private fun consoleLog(s: String) {
    js("console.log(s)")
}

@OptIn(ExperimentalWasmJsInterop::class)
private fun postLine(s: String) {
    js(
        """
        try {
            fetch('/bench', { method: 'POST', headers: { 'Content-Type': 'text/plain' }, body: s });
        } catch (e) {}
        """
    )
}

actual fun emitBench(line: String) {
    consoleLog(line)
    postLine(line)
}

actual val platformLabel: String = "wasmJs/" + window.navigator.userAgent.take(90)

@OptIn(ExperimentalWasmJsInterop::class)
private fun probeBackend(): String = js(
    """
    (function () {
        var out = [];
        var seen = [];
        function walk(root, depth) {
            if (!root || depth > 6) return;
            var kids = root.children || [];
            for (var i = 0; i < kids.length; i++) {
                var el = kids[i];
                if (el.tagName === 'CANVAS') seen.push(el);
                if (el.shadowRoot) walk(el.shadowRoot, depth + 1);
                walk(el, depth + 1);
            }
        }
        walk(document.body, 0);
        out.push('canvases=' + seen.length);
        out.push('bodyChildren=' + Array.prototype.map.call(document.body.children, function (e) {
            return e.tagName.toLowerCase() + (e.id ? '#' + e.id : '');
        }).join(','));
        for (var i = 0; i < seen.length; i++) {
            var c = seen[i];
            var kind = 'no-context';
            var extra = '';
            try {
                if (c.getContext('webgpu')) { kind = 'webgpu'; }
            } catch (e) {}
            try {
                if (kind === 'no-context' && c.getContext('webgl2')) {
                    kind = 'webgl2';
                    var gl = c.getContext('webgl2');
                    var dbg = gl.getExtension('WEBGL_debug_renderer_info');
                    extra = ' renderer=' + (dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER));
                }
            } catch (e) {}
            try {
                if (kind === 'no-context' && c.getContext('webgl')) {
                    kind = 'webgl1';
                    var g1 = c.getContext('webgl');
                    var d1 = g1.getExtension('WEBGL_debug_renderer_info');
                    extra = ' renderer=' + (d1 ? g1.getParameter(d1.UNMASKED_RENDERER_WEBGL) : g1.getParameter(g1.RENDERER));
                }
            } catch (e) {}
            try {
                if (kind === 'no-context' && c.getContext('2d')) { kind = 'canvas2d'; }
            } catch (e) {}
            out.push('canvas' + i + '=' + kind + extra + ' css=' + c.width + 'x' + c.height);
        }
        out.push('webgpu_api=' + (navigator.gpu ? 'present' : 'absent'));
        out.push('dpr=' + window.devicePixelRatio);
        return out.join(' | ');
    })()
    """
)

actual fun graphicsBackend(): String = probeBackend()
