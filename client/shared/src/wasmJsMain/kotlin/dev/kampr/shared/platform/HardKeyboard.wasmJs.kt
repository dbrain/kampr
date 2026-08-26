package dev.kampr.shared.platform

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import kotlin.js.ExperimentalWasmJsInterop

// Whether focusing an offscreen input would raise a keyboard over the page. A desk browser has none
// to raise; a touch browser must be left alone, because focusing is what raises the keys.
//
// Two readings rather than the usual `(hover: hover) and (pointer: fine)`, because that one is
// measured to answer "no device at all" — ChromeHeadless 151 reports `hover: none`, `pointer: none`
// and `pointer: coarse: false` together — and a browser that reports nothing has to fall on the
// side that keeps working. `maxTouchPoints` is a hardware count no phone gets wrong, and it is
// what makes "reports nothing" mean a desk rather than an unknown. A laptop with a touchscreen is
// read as touch, and the keydown latch below is what takes it back.
@OptIn(ExperimentalWasmJsInterop::class)
fun touchBrowser(): Boolean = js(
    """
    (function () {
        try {
            if ((navigator.maxTouchPoints || 0) > 0) return true;
            return window.matchMedia('(pointer: coarse)').matches;
        } catch (e) {
            return true;
        }
    })()
    """
)

// There is no browser API for "is a physical keyboard attached". `navigator.keyboard` is the
// Keyboard Map and Keyboard Lock API — it answers what a key prints and whether the page may take
// Escape — and it is present on Chrome for Android with no keyboard anywhere near the device, so
// it is not the question. Nothing else comes close, so what is left is evidence.
//
// The evidence is a keydown that a soft keyboard does not produce. Android IMEs report keyCode 229
// and `key: "Unidentified"` for printable text, and the handful of keys they do send for real are
// Enter and Backspace — so those three are excluded and everything else here is a key that has to
// have been pressed on something with keys: a modifier held on its own, or a key no on-screen
// keyboard in this layout draws at all.
//
// A latch rather than a reading, because it can only be honest in one direction. A keydown proves
// a keyboard was there; silence proves nothing, and no event fires when one is unplugged. So it
// goes `false → true` once and stays, which is the direction that never takes an operator's only
// Escape key away from them mid-session.
//
// Capturing, on the document, so it sees the key wherever the focus is — the offscreen input the
// terminal installs consumes what it handles, and this must not depend on having got there first.
//
// No `isTrusted` guard. Nothing in Kampr dispatches a KeyboardEvent — the key row's caps go over
// the wire as bytes, not as DOM events — so it would be guarding against a thing that does not
// happen, and Chrome 151 makes `isTrusted` a non-configurable own property of every event, so a
// test cannot forge one either. Keeping it would have made the whole latch unreachable from the
// browser harness, which is a test that proves nothing about the app.
@OptIn(ExperimentalWasmJsInterop::class)
internal fun hardKeySeen(): Boolean = js(
    """
    (function () {
        var s = globalThis.__kamprHardKeys;
        if (!s) {
            s = { seen: false };
            var named = /^(Escape|Tab|CapsLock|Control|Alt|Meta|Insert|Home|End|PageUp|PageDown|Arrow(Up|Down|Left|Right)|F([1-9]|1[0-9]|2[0-4]))$/;
            document.addEventListener('keydown', function (e) {
                if (s.seen || e.isComposing || e.keyCode === 229) return;
                var k = e.key;
                if (!k || k === 'Unidentified') return;
                if (e.ctrlKey || e.altKey || e.metaKey || named.test(k)) s.seen = true;
            }, true);
            globalThis.__kamprHardKeys = s;
        }
        return s.seen;
    })()
    """
)

// Polled rather than pushed, the way the on-screen keyboard inset is: the listener fires on a
// thread of the browser's choosing and there is no Compose-side channel to hand the value to. One
// boolean read a frame, and the loop ends the moment it latches, so a tablet costs a read per frame
// until its keyboard is used and nothing after that.
@Composable
actual fun hardKeyboardAttached(): Boolean {
    var attached by remember { mutableStateOf(!touchBrowser()) }
    LaunchedEffect(Unit) {
        while (!attached) {
            withFrameNanos { }
            if (hardKeySeen()) attached = true
        }
    }
    return attached
}
