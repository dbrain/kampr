package dev.kampr.terminal

import kotlin.js.ExperimentalWasmJsInterop

@OptIn(ExperimentalWasmJsInterop::class)
internal fun inputFocused(): Boolean =
    js("(function(){ var s = globalThis.__kamprInput; return !!(s && document.activeElement === s.el); })()")

// Karma's own browser is a headless Chromium reporting no input device at all, so a desk and a
// phone both have to be handed to the code as the readings they give. `dev.kampr.shared` measures
// the reading itself; what is faked here is only enough of it to pick a side.
@OptIn(ExperimentalWasmJsInterop::class)
internal fun pretendPointer(hover: String, pointer: String) {
    js(
        """
        (function () {
            if (!globalThis.__kamprRealMatchMedia) globalThis.__kamprRealMatchMedia = window.matchMedia;
            Object.defineProperty(window, 'matchMedia', {
                value: function (q) {
                    var ok = true;
                    var re = /\((hover|pointer)\s*:\s*([a-z]+)\)/g;
                    var m;
                    while ((m = re.exec(q)) !== null) {
                        if ((m[1] === 'hover' ? hover : pointer) !== m[2]) ok = false;
                    }
                    return { matches: ok, media: q };
                },
                configurable: true,
            });
        })()
        """
    )
}

@OptIn(ExperimentalWasmJsInterop::class)
internal fun stopPretending() {
    js(
        """
        (function () {
            if (globalThis.__kamprRealMatchMedia) {
                Object.defineProperty(window, 'matchMedia', {
                    value: globalThis.__kamprRealMatchMedia, configurable: true,
                });
            }
        })()
        """
    )
}

internal fun pretendDesk() = pretendPointer(hover = "hover", pointer = "fine")

internal fun pretendPhone() = pretendPointer(hover = "none", pointer = "coarse")

// A hardware chord as the browser reports one, and whether the page got to keep its default. A
// keydown the handler did not `preventDefault` is a keydown the browser is still free to act on,
// which for `⌘T` is the whole point.
@OptIn(ExperimentalWasmJsInterop::class)
internal fun chordKey(key: String, ctrl: Boolean, meta: Boolean, shift: Boolean): Boolean =
    js(
        """
        (function () {
            var s = globalThis.__kamprInput;
            var e = new KeyboardEvent('keydown', {
                key: key, ctrlKey: ctrl, metaKey: meta, shiftKey: shift,
                bubbles: true, cancelable: true,
            });
            s.el.dispatchEvent(e);
            return !e.defaultPrevented;
        })()
        """
    )
