package dev.kampr.shared

import dev.kampr.shared.platform.hardKeySeen
import dev.kampr.shared.platform.touchBrowser
import kotlin.js.ExperimentalWasmJsInterop
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

@OptIn(ExperimentalWasmJsInterop::class)
private fun pretendTouchscreen(points: Int) {
    js("Object.defineProperty(navigator, 'maxTouchPoints', { value: points, configurable: true })")
}

// The latch is one-way by design, so every test starts by putting it back. Reaching into the
// singleton rather than adding a reset to the production side: `hardKeySeen()` installs the
// listener on its first call, and the flag it sets is the whole of the state.
@OptIn(ExperimentalWasmJsInterop::class)
private fun clearLatch(): Unit = js("globalThis.__kamprHardKeys.seen = false")

private fun forgetKeyboard() {
    hardKeySeen()
    clearLatch()
}

@OptIn(ExperimentalWasmJsInterop::class)
private fun press(key: String, code: Int, ctrl: Boolean) {
    js(
        """
        (function () {
            var e = new KeyboardEvent('keydown', { key: key, bubbles: true, cancelable: true, ctrlKey: ctrl });
            Object.defineProperty(e, 'keyCode', { value: code });
            document.dispatchEvent(e);
        })()
        """
    )
}

private fun soft(key: String, code: Int = 229) = press(key, code, ctrl = false)

private fun hard(key: String, ctrl: Boolean = false) = press(key, 0, ctrl = ctrl)

// Runs in a real ChromeHeadless, which is the only place the browser half of this exists.
//
// There is no browser API for "is a physical keyboard attached" — `navigator.keyboard` is the
// Keyboard Map and Lock API, it answers what a key prints and whether the page may take Escape,
// and Chrome for Android exposes it with no keyboard within a metre of the device. So this is a
// heuristic, and what these tests pin down is the direction it fails in.
class BrowserKeyboardTest {
    @Test
    fun aBrowserThatReportsNoTouchAtAllIsTakenForADeskRatherThanAnUnknown() {
        pretendTouchscreen(0)
        assertFalse(touchBrowser(), "a browser reporting no touch points was still read as a touchscreen")
    }

    @Test
    fun aTouchBrowserIsNotCreditedWithAKeyboardItHasNeverShown() {
        pretendTouchscreen(5)
        forgetKeyboard()
        assertTrue(touchBrowser(), "a touchscreen reading was not believed")
        assertFalse(hardKeySeen(), "a browser that has seen no key at all claimed a keyboard")
    }

    // Every keydown an Android IME actually produces. `Unidentified` at keyCode 229 is every
    // printable character, and Enter and Backspace are the two it sends for real — so a tablet
    // that has been typed on all afternoon must still be holding its key row.
    @Test
    fun theKeydownsASoftKeyboardSendsNeverPromoteATabletToADesk() {
        pretendTouchscreen(5)
        forgetKeyboard()
        soft("Unidentified")
        soft("a")
        soft("Enter", code = 13)
        soft("Backspace", code = 8)
        soft("Process")
        assertFalse(hardKeySeen(), "a soft keyboard's own keys took the key row away from a tablet")
    }

    // The self-correcting half: a keyboard plugged into a tablet announces itself the first time
    // it is used, with a key no on-screen keyboard in this layout draws.
    @Test
    fun aKeyOnlyARealKeyboardHasSaysThereIsARealKeyboard() {
        for (key in listOf("Escape", "Tab", "ArrowUp", "F5", "Control", "PageDown")) {
            pretendTouchscreen(5)
            forgetKeyboard()
            hard(key)
            assertTrue(hardKeySeen(), "$key was pressed and the browser still thought it had no keyboard")
        }
    }

    @Test
    fun aModifierHeldOnAPrintableKeyIsAKeyboardToo() {
        pretendTouchscreen(5)
        forgetKeyboard()
        hard("c", ctrl = true)
        assertTrue(hardKeySeen(), "ctrl-c was pressed and the browser still thought it had no keyboard")
    }

    // Once, and it stays: nothing fires when a keyboard is unplugged, so the only honest reading
    // afterwards is the one that keeps working — and taking the row away mid-session on a guess is
    // the failure that costs an operator their Escape key.
    @Test
    fun theLatchDoesNotComeUndoneWhenTheNextKeyIsAnOrdinaryOne() {
        pretendTouchscreen(5)
        forgetKeyboard()
        hard("Escape")
        soft("Unidentified")
        soft("a")
        assertTrue(hardKeySeen(), "the keyboard reading came undone on the next soft keypress")
    }
}
