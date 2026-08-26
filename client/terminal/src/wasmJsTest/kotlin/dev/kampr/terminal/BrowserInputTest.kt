package dev.kampr.terminal

import dev.kampr.shared.platform.touchBrowser
import dev.kampr.terminal.input.drainInput
import dev.kampr.terminal.input.focusInput
import dev.kampr.terminal.input.holdsInput
import dev.kampr.terminal.input.installInput
import kotlin.js.ExperimentalWasmJsInterop
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

@OptIn(ExperimentalWasmJsInterop::class)
private fun inputFocused(): Boolean =
    js("(function(){ var s = globalThis.__kamprInput; return !!(s && document.activeElement === s.el); })()")

// Karma's own browser is a headless Chromium reporting no input device at all, so a desk and a
// phone both have to be handed to the code as the readings they give. `dev.kampr.shared` measures
// the reading itself; what is faked here is only enough of it to pick a side.
@OptIn(ExperimentalWasmJsInterop::class)
private fun pretendPointer(hover: String, pointer: String) {
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
private fun stopPretending() {
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

private fun pretendDesk() = pretendPointer(hover = "hover", pointer = "fine")

private fun pretendPhone() = pretendPointer(hover = "none", pointer = "coarse")

@OptIn(ExperimentalWasmJsInterop::class)
private fun stealFocus() {
    js("(function () { var s = globalThis.__kamprInput; if (s) s.el.blur(); })()")
}

@OptIn(ExperimentalWasmJsInterop::class)
private fun typeKey(key: String) {
    js(
        """
        (function () {
            var s = globalThis.__kamprInput;
            s.el.dispatchEvent(new KeyboardEvent('keydown', { key: key, bubbles: true, cancelable: true }));
        })()
        """
    )
}

// Runs in a real ChromeHeadless, which is the only place the browser half of the input exists.
// Everything here is the DOM the wasm build actually talks to, not a model of it.
class BrowserInputTest {
    @Test
    fun theOffscreenInputCanBeGivenAndTakenTheKeyboardFocus() {
        installInput()
        focusInput(false)
        assertFalse(inputFocused(), "blurring left the offscreen input holding the keyboard")
        focusInput(true)
        assertTrue(inputFocused(), "the offscreen input refused the focus the pane gave it")
        focusInput(false)
    }

    // The report: a browser terminal that shows frames and takes no keys. The desktop layout
    // carries no key row, so `keyboardOpen` is never set, so the input was never focused and
    // every keystroke went to the page instead of the pane.
    @Test
    fun aDeskBrowserHoldsTheInputWithNobodyHavingAskedForAKeyboard() {
        try {
            pretendDesk()
            assertFalse(touchBrowser(), "this browser reported a touchscreen it does not have")
            assertTrue(
                holdsInput(enabled = true, keyboardAsked = false, touch = touchBrowser()),
                "a pane on a desk browser has to hold the input before anything is asked for",
            )
        } finally {
            stopPretending()
        }
    }

    // The half of the split that lives here. The key row and the focus ask two different questions
    // of one browser, and a browser reporting no input device answers neither — so they are allowed
    // to disagree, and they do. The key row offers itself, because nothing said there is a
    // keyboard. The focus is held, because there is no soft keyboard to raise over the pane and a
    // pane that refuses the focus takes no keys at all. Sharing one answer is what made this a
    // choice between a desk and a phone for a machine that is neither.
    @Test
    fun aBrowserThatReportsNoInputDeviceAtAllStillHoldsTheInput() {
        stopPretending()
        assertFalse(touchBrowser(), "the headless harness reported a touchscreen it does not have")
        assertTrue(
            holdsInput(enabled = true, keyboardAsked = false, touch = touchBrowser()),
            "a browser with no keyboard to raise over the pane still refused the focus",
        )
    }

    // The guard: focusing is what raises the keys, so a touch browser must be left alone until
    // the operator asks. A coarse primary pointer is the reading a phone gets right, and this is
    // it, faked.
    @Test
    fun aTouchBrowserIsLeftAloneUntilTheOperatorAsksForAKeyboard() {
        try {
            pretendPhone()
            assertTrue(touchBrowser(), "a touchscreen reading was not believed")
            assertFalse(
                holdsInput(enabled = true, keyboardAsked = false, touch = touchBrowser()),
                "a pane opening on a phone browser raised the keyboard over itself",
            )
            assertTrue(
                holdsInput(enabled = true, keyboardAsked = true, touch = touchBrowser()),
                "the keyboard cap stopped working on a phone browser",
            )
        } finally {
            stopPretending()
        }
    }

    // Which is also what keeps the mosaic honest: every cell but the focused one is handed a
    // read-only `PaneIo`, so a screen full of terminals has exactly one that can take the keys.
    @Test
    fun aReadOnlyPaneNeverHoldsTheInput() {
        assertFalse(holdsInput(enabled = false, keyboardAsked = true, touch = false))
        assertFalse(holdsInput(enabled = false, keyboardAsked = false, touch = false))
    }

    // A pointer down on the canvas blurs the offscreen input, and a desk browser has no keyboard
    // request to notice — so a scroll on the grid left the pane deaf. `reclaimKeyboard` is the
    // cue, and it has to fire whether or not a keyboard was ever asked for.
    @Test
    fun aGestureThatBlurredTheInputEndsWithTheFocusBack() {
        installInput()
        focusInput(true)
        stealFocus()
        assertFalse(inputFocused(), "the harness never took the focus away")

        val session = PaneSession("01JKAMPRNODE0000000000000/w1:p1")
        val before = session.surfaceSettled
        session.reclaimKeyboard()
        assertTrue(session.surfaceSettled > before, "a settled gesture told nobody about itself")

        focusInput(holdsInput(true, session.keyboardOpen, touchBrowser()))
        assertTrue(inputFocused(), "the pane never took back the focus the gesture cost it")
        focusInput(false)
    }

    @Test
    fun aKeyPressedOnTheFocusedInputArrivesAsItsEscapeSequence() {
        installInput()
        focusInput(true)
        drainInput()
        typeKey("Enter")
        assertEquals("\r", drainInput(), "a hardware Enter never reached the pane")
        typeKey("ArrowUp")
        assertEquals("\u001b[A", drainInput(), "a hardware arrow never reached the pane")
        focusInput(false)
    }
}
