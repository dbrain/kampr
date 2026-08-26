package dev.kampr.shared

import dev.kampr.shared.platform.PointerKind
import dev.kampr.shared.platform.deskBrowser
import dev.kampr.shared.platform.pointerKind
import dev.kampr.shared.platform.touchBrowser
import kotlin.js.ExperimentalWasmJsInterop
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

// A browser other than the one the harness is. `matchMedia` answers for the machine Karma happens
// to be running on and that machine is a headless Chromium with no input device — so every real
// environment this reading has to be right about is one the harness cannot be, and the only way to
// put them in front of the code is to hand it their readings.
//
// `maxTouchPoints` is faked alongside, and it is faked *against* the media query on purpose: it is
// what the previous reading believed and what read an operator's desk as a phone, so a row that
// sets it high while reporting a fine pointer is the one that fails if it ever comes back.
@OptIn(ExperimentalWasmJsInterop::class)
private fun pretendBrowser(hover: String, pointer: String, touchPoints: Int) {
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
            Object.defineProperty(navigator, 'maxTouchPoints', { value: touchPoints, configurable: true });
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
            Object.defineProperty(navigator, 'maxTouchPoints', { value: 0, configurable: true });
        })()
        """
    )
}

private class Browser(val name: String, val hover: String, val pointer: String, val touchPoints: Int, val kind: PointerKind)

// Every one of these is a machine the app is actually opened on, described by what its browser
// reports rather than by what would be convenient. The desk rows are the report this exists for:
// a keyboard row appeared on a machine with a keyboard, because something on it counts touch
// points.
private val BROWSERS = listOf(
    Browser("a desktop browser driven by a mouse", "hover", "fine", 0, PointerKind.Desk),
    Browser("a desktop whose driver reports a digitiser", "hover", "fine", 10, PointerKind.Desk),
    Browser("a laptop with a touchscreen, driven by its trackpad", "hover", "fine", 5, PointerKind.Desk),
    Browser("a phone browser", "none", "coarse", 5, PointerKind.Touch),
    Browser("a tablet browser", "none", "coarse", 10, PointerKind.Touch),
    Browser("a browser reporting no input device at all", "none", "none", 0, PointerKind.Unknown),
)

// Runs in a real ChromeHeadless, which is the only place the browser half of this exists.
class BrowserKeyboardTest {
    @Test
    fun theBrowsersThisIsOpenedOnAreReadAsTheMachinesTheyAre() {
        try {
            for (browser in BROWSERS) {
                pretendBrowser(browser.hover, browser.pointer, browser.touchPoints)
                assertEquals(browser.kind, pointerKind(), "${browser.name} was read as the wrong kind of machine")
            }
        } finally {
            stopPretending()
        }
    }

    // The report, verbatim: a desk browser showed the key row, and a keypress later took it away.
    // The row should never have been there, and this is the half of that which is a reading.
    @Test
    fun aDeskIsNotCalledATouchscreenByTheTouchPointsItsDriverCountsForIt() {
        try {
            pretendBrowser(hover = "hover", pointer = "fine", touchPoints = 10)
            assertFalse(touchBrowser(), "a desktop reporting a digitiser's touch points was read as a phone")
            assertTrue(deskBrowser(), "a desktop driven by a mouse was not credited with the keyboard under it")
        } finally {
            stopPretending()
        }
    }

    // Not broken by the fix: focusing is what raises a soft keyboard, and a phone must still be
    // left alone and still be offered the row it types Ctrl-C with.
    @Test
    fun aPhoneBrowserIsStillATouchscreenAndStillHasNoKeyboard() {
        try {
            pretendBrowser(hover = "none", pointer = "coarse", touchPoints = 5)
            assertTrue(touchBrowser(), "a phone browser was not read as a touchscreen")
            assertFalse(deskBrowser(), "a phone browser was credited with a hardware keyboard")
        } finally {
            stopPretending()
        }
    }

    // The third answer, and the reason there are three. It is neither of the other two, and the
    // two callers are allowed to disagree about what to do with it — which they do.
    @Test
    fun aBrowserWithNoInputDeviceIsNeitherADeskNorATouchscreen() {
        try {
            pretendBrowser(hover = "none", pointer = "none", touchPoints = 0)
            assertFalse(deskBrowser(), "a browser that reported no input device claimed a keyboard")
            assertFalse(touchBrowser(), "a browser that reported no input device claimed a touchscreen")
        } finally {
            stopPretending()
        }
    }

    // The harness itself, unfaked and measured: ChromeHeadless 151 reports `hover: none`,
    // `pointer: none`, `any-pointer: none` and `maxTouchPoints: 0`. It is the `Unknown` case, and
    // naming it here is what keeps it from being mistaken for a user environment again — the
    // previous reading was chosen to make this machine read as a desk, and that is what put the
    // key row on a real one.
    @Test
    fun theTestHarnessIsTheBrowserThatReportsNothingAndIsNotAMachineAnyoneSitsAt() {
        stopPretending()
        assertEquals(PointerKind.Unknown, pointerKind(), "ChromeHeadless stopped reporting no input device")
    }
}
