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

@OptIn(ExperimentalWasmJsInterop::class)
private fun pretendTouchscreen(points: Int) {
    js("Object.defineProperty(navigator, 'maxTouchPoints', { value: points, configurable: true })")
}

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
        pretendTouchscreen(0)
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
        pretendTouchscreen(0)
        assertFalse(touchBrowser(), "this browser reported a touchscreen it does not have")
        assertTrue(
            holdsInput(enabled = true, keyboardAsked = false, touch = touchBrowser()),
            "a pane on a desk browser has to hold the input before anything is asked for",
        )
    }

    // The guard: focusing is what raises the keys, so a touch browser must be left alone until
    // the operator asks. `maxTouchPoints` is the reading a phone gets right, and this is it, faked.
    @Test
    fun aTouchBrowserIsLeftAloneUntilTheOperatorAsksForAKeyboard() {
        pretendTouchscreen(5)
        assertTrue(touchBrowser(), "a touchscreen reading was not believed")
        assertFalse(
            holdsInput(enabled = true, keyboardAsked = false, touch = touchBrowser()),
            "a pane opening on a phone browser raised the keyboard over itself",
        )
        assertTrue(
            holdsInput(enabled = true, keyboardAsked = true, touch = touchBrowser()),
            "the keyboard cap stopped working on a phone browser",
        )
        pretendTouchscreen(0)
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
        pretendTouchscreen(0)
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
        pretendTouchscreen(0)
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
