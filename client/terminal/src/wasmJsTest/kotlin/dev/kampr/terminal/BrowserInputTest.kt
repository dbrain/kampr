package dev.kampr.terminal

import dev.kampr.shared.platform.touchBrowser
import dev.kampr.terminal.input.deliverInputTo
import dev.kampr.terminal.input.drainInput
import dev.kampr.terminal.input.focusInput
import dev.kampr.terminal.input.holdsInput
import dev.kampr.terminal.input.deliverChordsTo
import dev.kampr.terminal.input.installInput
import dev.kampr.terminal.input.PaneChord
import dev.kampr.terminal.input.reclaimInputFocus
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

// What a printable key produces on this path: an IME and a soft keyboard both report keyCode 229
// to keydown, so `beforeinput` is the one that carries the character.
@OptIn(ExperimentalWasmJsInterop::class)
private fun typeText(data: String) {
    js(
        """
        (function () {
            var s = globalThis.__kamprInput;
            s.el.dispatchEvent(new InputEvent('beforeinput', {
                inputType: 'insertText', data: data, bubbles: true, cancelable: true,
            }));
        })()
        """
    )
}

@OptIn(ExperimentalWasmJsInterop::class)
private fun composeKey(type: String, data: String) {
    js(
        """
        (function () {
            var s = globalThis.__kamprInput;
            s.el.dispatchEvent(new CompositionEvent(type, { data: data, bubbles: true }));
        })()
        """
    )
}

// A hidden tab, as far as anything in the client can tell one: the document says hidden, and
// `requestAnimationFrame` takes the callback and never calls it. ChromeHeadless cannot be hidden
// for real — measured on Chrome 151: opening a popup leaves this page `visible` and still ticking
// (32 frames in 500 ms), and a `display:none` iframe gets its frames too — so the two things the
// browser does to a hidden tab are done to it here instead.
@OptIn(ExperimentalWasmJsInterop::class)
private fun parkTheTab() {
    js(
        """
        (function () {
            if (globalThis.__kamprRealRaf) return;
            globalThis.__kamprRealRaf = window.requestAnimationFrame;
            globalThis.__kamprFramesAsked = 0;
            window.requestAnimationFrame = function () { globalThis.__kamprFramesAsked++; return 0; };
            Object.defineProperty(document, 'hidden', { value: true, configurable: true });
            Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
            document.dispatchEvent(new Event('visibilitychange'));
        })()
        """
    )
}

@OptIn(ExperimentalWasmJsInterop::class)
private fun showTheTab() {
    js(
        """
        (function () {
            if (!globalThis.__kamprRealRaf) return;
            window.requestAnimationFrame = globalThis.__kamprRealRaf;
            globalThis.__kamprRealRaf = null;
            delete document.hidden;
            delete document.visibilityState;
            document.dispatchEvent(new Event('visibilitychange'));
        })()
        """
    )
}

@OptIn(ExperimentalWasmJsInterop::class)
private fun framesAskedFor(): Int = js("globalThis.__kamprFramesAsked || 0")


// A rival for the one focus slot the page has. A `textarea` is the shape Compose's own text input
// takes in this DOM, so it is what the conversation composer looks like from here; a `button` is
// every other focusable thing on the page.
@OptIn(ExperimentalWasmJsInterop::class)
private fun putRival(tag: String) {
    js(
        """
        (function () {
            var old = globalThis.__kamprRival;
            if (old && old.parentNode) old.parentNode.removeChild(old);
            var el = document.createElement(tag);
            if (tag === 'button') el.textContent = 'x';
            document.body.appendChild(el);
            globalThis.__kamprRival = el;
            el.focus();
        })()
        """
    )
}

@OptIn(ExperimentalWasmJsInterop::class)
private fun dismissRival() {
    js(
        """
        (function () {
            var el = globalThis.__kamprRival;
            if (!el) return;
            el.blur();
            if (el.parentNode) el.parentNode.removeChild(el);
            globalThis.__kamprRival = null;
        })()
        """
    )
}


// A hardware chord as the browser reports one, and whether the page got to keep its default. A
// keydown the handler did not `preventDefault` is a keydown the browser is still free to act on,
// which for `⌘T` is the whole point.
@OptIn(ExperimentalWasmJsInterop::class)
private fun chordKey(key: String, ctrl: Boolean, meta: Boolean, shift: Boolean): Boolean =
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


    // The report: a browser pane kept painting frames from the desk and stopped taking keys, for
    // good. DOM focus is one slot for the whole page and the pane asserted its claim on it exactly
    // once, from a `LaunchedEffect` keyed on four things a composer never moves — so whatever took
    // the slot next kept it, and the pane went deaf with nothing complaining. The operator had
    // just used the conversation composer, which is exactly this.
    @Test
    fun aComposerThatTookTheKeyboardHandsItBackWhenItIsDoneWithIt() {
        installInput()
        focusInput(true)
        assertTrue(inputFocused(), "the pane never held the keyboard to begin with")

        putRival("textarea")
        assertFalse(inputFocused(), "the harness never moved the focus off the pane")
        reclaimInputFocus()
        assertFalse(inputFocused(), "the pane stole the keyboard back from a live text field")

        dismissRival()
        reclaimInputFocus()
        assertTrue(inputFocused(), "the pane never took the keyboard back after the composer closed")
        focusInput(false)
    }

    // Not every rival for the focus gives it up again. A button keeps it until something else asks,
    // and nothing was asking — so a single tap on the pane chrome cost the pane its keyboard with
    // no way back that did not involve touching the grid.
    @Test
    fun aButtonThatTookTheFocusDoesNotCostThePaneItsKeyboard() {
        installInput()
        focusInput(true)
        putRival("button")
        assertFalse(inputFocused(), "the harness never moved the focus off the pane")
        reclaimInputFocus()
        assertTrue(inputFocused(), "a press on the pane chrome left the pane deaf")
        dismissRival()
        focusInput(false)
    }

    // The guard on the renewal: it renews a claim, it never makes one. A phone browser is left
    // alone until the operator asks for a keyboard, and a per-frame reclaim that ignored that
    // would raise the soft keyboard over every pane the moment it opened.
    @Test
    fun aReclaimRenewsAClaimAndNeverMakesOne() {
        installInput()
        focusInput(false)
        reclaimInputFocus()
        assertFalse(inputFocused(), "a reclaim raised a keyboard nobody had asked for")
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

    // The report: every keystroke waited up to a frame — 16.7 ms at 60 Hz — before it was even
    // put on the wire, because the queue was emptied from a `withFrameNanos` loop and that is
    // `requestAnimationFrame`. #273 puts herdr's own share of the round trip at 1-3 ms and the
    // rest in the program the pane runs, which leaves this the only share Kampr had to give back.
    // So the frames are taken away outright here: delivery that needs one does not deliver.
    @Test
    fun aKeystrokeReachesThePaneWithNoAnimationFrameHavingRun() {
        installInput()
        focusInput(true)
        drainInput()
        val typed = StringBuilder()
        deliverInputTo { typed.append(it) }
        try {
            parkTheTab()
            typeKey("Enter")
            assertEquals("\r", typed.toString(), "a keystroke was still waiting for a frame")
            typeText("x")
            assertEquals("\rx", typed.toString(), "a typed character was still waiting for a frame")
            assertEquals(0, framesAskedFor(), "delivering a keystroke asked for an animation frame")
        } finally {
            showTheTab()
            deliverInputTo(null)
            focusInput(false)
        }
    }

    // The report: a tab left in the background swallows what is typed at it and hands the lot over
    // at once when it is looked at again, with nothing saying so. A browser stops calling
    // `requestAnimationFrame` back for a hidden tab, and the drain was riding it — so the queue
    // parked exactly as long as the tab was away. Delivery now happens inside the DOM event, which
    // a hidden tab does not throttle, and there is no clock left to park.
    @Test
    fun aTabThatIsHiddenDeliversWhatIsTypedAtItInsteadOfHoardingItUntilItIsShown() {
        installInput()
        focusInput(true)
        drainInput()
        val typed = StringBuilder()
        deliverInputTo { typed.append(it) }
        try {
            parkTheTab()
            typeText("l")
            typeText("s")
            typeKey("Enter")
            assertEquals("ls\r", typed.toString(), "a hidden tab hoarded what was typed at it")
            showTheTab()
            assertEquals("ls\r", typed.toString(), "the tab coming back delivered something twice")
        } finally {
            showTheTab()
            deliverInputTo(null)
            focusInput(false)
        }
    }

    // Ordering is the only reason the queue is still there. An IME's `compositionupdate` fires
    // between the `beforeinput` of the keys typed around it, and the diff against what was
    // composed last is only correct read in the order it arrived — so delivering per event rather
    // than per frame has to produce the same bytes, in the same order, once each.
    @Test
    fun anImeCommitBetweenTwoTypedKeysArrivesInTheOrderItWasTyped() {
        installInput()
        focusInput(true)
        drainInput()
        val typed = StringBuilder()
        deliverInputTo { typed.append(it) }
        try {
            typeText("a")
            composeKey("compositionstart", "")
            composeKey("compositionupdate", "k")
            composeKey("compositionupdate", "ka")
            composeKey("compositionupdate", "\u304b")
            composeKey("compositionend", "\u304b")
            typeText("b")
            typeKey("Enter")
            assertEquals(
                "aka\u007f\u007f\u304bb\r",
                typed.toString(),
                "the pane was sent something other than what was typed, in some other order",
            )
        } finally {
            deliverInputTo(null)
            focusInput(false)
        }
    }

    // The seam the frame loop used to cover: it came round again whoever was listening, so a
    // keystroke that landed between two panes was picked up by the next turn. Nothing comes round
    // any more, so the handover itself has to carry what is waiting.
    @Test
    fun aKeystrokeThatLandedWithNoPaneToTakeItGoesToTheNextPaneRatherThanNowhere() {
        installInput()
        focusInput(true)
        drainInput()
        deliverInputTo(null)
        typeKey("ArrowUp")
        val typed = StringBuilder()
        try {
            deliverInputTo { typed.append(it) }
            assertEquals("\u001b[A", typed.toString(), "a keystroke typed between two panes was stranded")
        } finally {
            deliverInputTo(null)
            focusInput(false)
        }
    }

    // The report: the copy chord was sent to the pane as a control code, so **copying interrupted
    // the process**. `ctrl+shift+C` arrives here as `e.key === "C"`, and the handler lowercased it
    // and looked it straight up in the control table. `⌘C` hit the same branch, which made
    // Command-C a SIGINT on macOS.
    //
    // This is the browser half, in a real browser: what the DOM handler queues, and what it lets
    // the page keep. The meaning of each chord is `paneChord`'s, tested against its own table.
    @Test
    fun theCopyAndPasteChordsAreTakenOffTheKeyPathInsteadOfBeingSentToThePane() {
        installInput()
        focusInput(true)
        drainInput()
        val chords = mutableListOf<PaneChord>()
        val typed = StringBuilder()
        deliverChordsTo { chords += it }
        deliverInputTo { typed.append(it) }
        try {
            for (chord in listOf("c" to PaneChord.Copy, "v" to PaneChord.Paste)) {
                chords.clear()
                typed.clear()
                assertFalse(
                    chordKey(chord.first.uppercase(), ctrl = true, meta = false, shift = true),
                    "ctrl+shift+${chord.first} left the page its default",
                )
                assertEquals(listOf(chord.second), chords, "ctrl+shift+${chord.first} reached nothing")
                assertEquals("", typed.toString(), "ctrl+shift+${chord.first} was sent to the pane as bytes")

                chords.clear()
                assertFalse(
                    chordKey(chord.first, ctrl = false, meta = true, shift = false),
                    "the command chord left the page its default",
                )
                assertEquals(listOf(chord.second), chords, "the command chord reached nothing")
                assertEquals("", typed.toString(), "the command chord was sent to the pane as bytes")
            }
        } finally {
            deliverChordsTo(null)
            deliverInputTo(null)
            focusInput(false)
        }
    }

    // And what must not change with it: an unshifted ctrl chord is still the control byte, which is
    // the whole of a terminal, and a shifted one that is not C or V still is too.
    @Test
    fun aPlainControlChordStillReachesThePaneAsItsByte() {
        installInput()
        focusInput(true)
        drainInput()
        val chords = mutableListOf<PaneChord>()
        val typed = StringBuilder()
        deliverChordsTo { chords += it }
        deliverInputTo { typed.append(it) }
        try {
            chordKey("c", ctrl = true, meta = false, shift = false)
            assertEquals("\u0003", typed.toString(), "ctrl+C stopped interrupting the pane")
            typed.clear()
            chordKey("A", ctrl = true, meta = false, shift = true)
            assertEquals("\u0001", typed.toString(), "ctrl+shift+A stopped producing its control byte")
            assertTrue(chords.isEmpty(), "a control chord was taken for a copy")
        } finally {
            deliverChordsTo(null)
            deliverInputTo(null)
            focusInput(false)
        }
    }

    // The other half of the same defect, and the one a browser feels: every `⌘`/Super chord with
    // a letter in it was turned into a control byte and `preventDefault`ed, so `⌘T`, `⌘W` and
    // `⌘L` both interrupted the pane and never reached the browser.
    @Test
    fun aCommandChordThatIsNotCopyOrPasteIsLeftEntirelyToTheBrowser() {
        installInput()
        focusInput(true)
        drainInput()
        val chords = mutableListOf<PaneChord>()
        val typed = StringBuilder()
        deliverChordsTo { chords += it }
        deliverInputTo { typed.append(it) }
        try {
            for (key in listOf("t", "w", "l", "r")) {
                assertTrue(
                    chordKey(key, ctrl = false, meta = true, shift = false),
                    "the browser's own command-$key was swallowed by the pane",
                )
            }
            assertEquals("", typed.toString(), "a command chord was sent to the pane as a control byte")
            assertTrue(chords.isEmpty(), "a command chord was taken for a copy")
        } finally {
            deliverChordsTo(null)
            deliverInputTo(null)
            focusInput(false)
        }
    }
}
