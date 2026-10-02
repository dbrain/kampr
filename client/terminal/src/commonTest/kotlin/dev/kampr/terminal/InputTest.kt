package dev.kampr.terminal

import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.input.CapKind
import dev.kampr.terminal.input.capHold
import dev.kampr.terminal.input.capPress
import dev.kampr.terminal.input.Esc
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.KeyLayouts
import dev.kampr.terminal.input.Latch
import dev.kampr.terminal.input.Latches
import dev.kampr.terminal.input.LatchState
import dev.kampr.terminal.input.active
import dev.kampr.terminal.input.PaneScroll
import dev.kampr.terminal.input.KeyTrace
import dev.kampr.terminal.input.ScrollTrace
import dev.kampr.terminal.input.ScrollKeys
import dev.kampr.terminal.input.land
import dev.kampr.terminal.input.PaneChord
import dev.kampr.terminal.input.chordSendsControl
import dev.kampr.terminal.input.paneChord
import dev.kampr.terminal.input.paneScrollKeys
import kotlin.test.Test
import kotlin.time.Duration.Companion.milliseconds
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

private class Recorder : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }

    override fun prefs(paneId: String) = PanePrefs()

    val text: List<String> get() = sent.filterIsInstance<ClientMsg.InputText>().map { it.text }
}

private fun sink(): Pair<Recorder, InputSink> {
    val recorder = Recorder()
    return recorder to InputSink("n/w1:p1", recorder, Latches())
}

private fun allCaps() = (KeyLayouts.portrait + KeyLayouts.portraitFn + KeyLayouts.landscape +
    KeyLayouts.landscapeFn).flatten().filterNotNull()
    .filter { it.kind != CapKind.Blank }
    .flatMap { listOfNotNull(it, it.alternate) }

class InputTest {
    // Everything the grid sends is typed by someone looking at where it lands, so none of it waits
    // on the node's boot hold, which is for a reply sent from the conversation view (#535).
    @Test
    fun everythingTheGridSendsIsMarkedTyped() {
        val (recorder, keys) = sink()
        keys.type("ls")
        keys.raw("\u001b[A")
        keys.paste("cargo test")
        assertTrue(recorder.sent.isNotEmpty(), "nothing was sent, so nothing is tested")
        assertTrue(
            recorder.sent.all { it is ClientMsg.InputText && it.typed },
            "the grid sent input the node would hold as a reply: ${recorder.sent}",
        )
    }

    // Probe: a pointer down anywhere on the canvas blurs the browser's offscreen input, so a cap
    // that sends its key and stops has also closed the keyboard, and everything typed next is
    // eaten with no signal.
    @Test
    fun everyCapKeepsTheKeyboardItsTapJustBlurred() {
        for (cap in allCaps().filter { it.kind != CapKind.Keyboard }) {
            val (_, keys) = sink()
            val session = PaneSession("n/w1:p1")
            session.openKeyboard()
            val before = session.focusRequests
            capPress(cap, session, keys)
            assertTrue(session.keyboardOpen, "${cap.label} closed the keyboard")
            assertTrue(session.focusRequests > before, "${cap.label} never claimed focus back")
            val held = session.focusRequests
            capHold(cap, session, keys)
            assertTrue(session.focusRequests > held, "holding ${cap.label} never claimed focus back")
        }
    }

    @Test
    fun aClosedKeyboardStaysClosedWhenACapSendsItsKey() {
        val (_, keys) = sink()
        val session = PaneSession("n/w1:p1")
        val tab = allCaps().first { it.label == "tab" }
        capPress(tab, session, keys)
        assertFalse(session.keyboardOpen, "a cap that sends its own key must not raise a keyboard")
    }

    // Ctrl and alt are prefixes and the key they take is nearly always a letter, which is the one
    // thing this row does not carry. Arming one with the keyboard down leaves a chord that cannot
    // be finished, so arming one is the ask.
    @Test
    fun armingCtrlOrAltRaisesTheKeyboardTheChordNeeds() {
        for (label in listOf("ctrl", "alt")) {
            val (_, keys) = sink()
            val session = PaneSession("n/w1:p1")
            val cap = allCaps().first { it.label == label }
            val before = session.focusRequests
            capPress(cap, session, keys)
            assertTrue(session.keyboardOpen, "$label armed with no keyboard to finish the chord")
            assertTrue(session.focusRequests > before, "$label never asked for focus")
        }
    }

    // The other half of the same rule. Shift rides the arrows and tab that are already on the row
    // and fn *is* the row, so neither is a request for letters; and clearing a modifier is the
    // opposite of asking for one.
    @Test
    fun onlyArmingALetterPrefixRaisesTheKeyboard() {
        val (_, keys) = sink()
        val ctrl = allCaps().first { it.label == "ctrl" }
        val alt = allCaps().first { it.label == "alt" }
        val fn = allCaps().first { it.label == "fn" }
        val shift = allCaps().first { it.latch == Latch.Shift }

        for (cap in listOf(shift, fn)) {
            val pressed = PaneSession("n/w1:p1")
            capPress(cap, pressed, keys)
            assertFalse(pressed.keyboardOpen, "${cap.label} takes no letter")
            val held = PaneSession("n/w1:p1")
            capHold(cap, held, keys)
            assertFalse(held.keyboardOpen, "holding ${cap.label} takes no letter")
        }

        // Neither prefix has a rider now that shift and fn have caps of their own, so holding one
        // locks it — and a locked prefix is the same unfinishable chord an armed one is, waiting on
        // a letter this row does not carry.
        for (cap in listOf(ctrl, alt)) {
            val locked = PaneSession("n/w1:p1")
            capHold(cap, locked, keys)
            assertTrue(locked.keyboardOpen, "a locked ${cap.label} is a chord with nothing on this row to finish it")
        }

        for ((cap, latch) in listOf(ctrl to Latch.Ctrl, alt to Latch.Alt)) {
            val session = PaneSession("n/w1:p1")
            session.latches.lock(latch)
            capPress(cap, session, keys)
            assertFalse(session.keyboardOpen, "clearing ${cap.label} asked for a keyboard")
        }
    }

    // The only affordance that names the keyboard is the one that has to be able to bring it back.
    @Test
    fun theKeyboardCapToggles() {
        val (_, keys) = sink()
        val session = PaneSession("n/w1:p1")
        val kbd = allCaps().first { it.kind == CapKind.Keyboard }
        session.openKeyboard()
        capPress(kbd, session, keys)
        assertFalse(session.keyboardOpen)
        val before = session.focusRequests
        capPress(kbd, session, keys)
        assertTrue(session.keyboardOpen, "there is no other way back to the keyboard")
        assertTrue(session.focusRequests > before, "reopening has to re-request focus")
    }

    @Test
    fun everyKeyHerdrRejectsGoesOutAsAnEscapeSequence() {
        val (recorder, keys) = sink()
        val caps = allCaps()
        val rejected = mapOf(
            "home" to Esc.HOME,
            "end" to Esc.END,
            "pgup" to Esc.PAGE_UP,
            "pgdn" to Esc.PAGE_DOWN,
            "ins" to Esc.INSERT,
            "del" to Esc.DELETE,
        )
        for ((label, sequence) in rejected) {
            val cap = caps.firstOrNull { it.label == label }
            assertTrue(cap != null, "$label is not reachable from any key row layout")
            keys.press(cap)
            assertEquals(sequence, recorder.text.last(), "$label sent the wrong sequence")
        }
    }

    @Test
    fun nothingInTheKeyRowUsesSendKeys() {
        val (recorder, keys) = sink()
        val caps = allCaps().filter { it.kind == CapKind.Text }
        for (cap in caps) keys.press(cap)
        assertTrue(recorder.sent.isNotEmpty())
        assertTrue(
            recorder.sent.all { it is ClientMsg.InputText },
            "the key row must never depend on send_keys",
        )
    }

    @Test
    fun latchesDecorateTheNextKeystrokeOnly() {
        val (recorder, keys) = sink()
        keys.latches.tap(Latch.Ctrl)
        keys.type("c")
        keys.type("c")
        assertEquals(listOf("\u0003", "c"), recorder.text)
    }

    @Test
    fun aLockedLatchKeepsApplying() {
        val (recorder, keys) = sink()
        keys.latches.lock(Latch.Ctrl)
        keys.type("a")
        keys.type("b")
        assertEquals(listOf("\u0001", "\u0002"), recorder.text)
    }

    @Test
    fun altPrefixesEscapeAndShiftShiftsSymbols() {
        val (recorder, keys) = sink()
        keys.latches.tap(Latch.Alt)
        keys.type("x")
        keys.latches.tap(Latch.Shift)
        keys.type("/")
        assertEquals(listOf("\u001bx", "?"), recorder.text)
    }

    @Test
    fun modifiersOnCsiSequencesUseTheXtermParameter() {
        assertEquals("\u001b[1;5A", Esc.modified(Esc.UP, ctrl = true, alt = false, shift = false))
        assertEquals("\u001b[5;3~", Esc.modified(Esc.PAGE_UP, ctrl = false, alt = true, shift = false))
        assertEquals(Esc.UP, Esc.modified(Esc.UP, ctrl = false, alt = false, shift = false))
    }

    @Test
    fun escapeSequencesFromHardwareKeysBypassTheLatches() {
        val (recorder, keys) = sink()
        keys.latches.tap(Latch.Ctrl)
        keys.type(Esc.PAGE_UP)
        assertEquals(listOf(Esc.PAGE_UP), recorder.text)
    }

    // The arrow cluster is an inverted T on every layer: up sits directly above down, with left
    // and right flanking it, so a thumb finds the key without looking.
    @Test
    fun theArrowsFormAnInvertedT() {
        for (rows in listOf(KeyLayouts.portrait, KeyLayouts.portraitFn, KeyLayouts.landscape, KeyLayouts.landscapeFn)) {
            val top = rows[0]
            val bottom = rows[1]
            val up = top.indexOfFirst { it?.label == "↑" }
            val down = bottom.indexOfFirst { it?.label == "↓" }
            // The column and not the slot: a cap that spans two columns is one entry in the row
            // and two of what a thumb sees, so counting entries stopped being the same question.
            assertEquals(columnOf(top, up), columnOf(bottom, down), "up must sit directly above down")
            assertEquals("←", bottom[down - 1]?.label)
            assertEquals("→", bottom[down + 1]?.label)
        }
    }

    // The operator: *"can we make the shift button two cols wide instead of having a blank
    // space"*. The blank beside shift was there to hold the navigation group's columns in line
    // with the row above, which is a job the separator can do instead — `KeyRowColumnsTest`
    // measures that it does.
    @Test
    fun shiftIsTwoColumnsWideRatherThanACapBesideAGap() {
        for ((name, rows) in listOf("portrait" to KeyLayouts.portrait, "landscape" to KeyLayouts.landscape)) {
            val row = rows.first { row -> row.any { it?.latch == Latch.Shift } }
            val modifiers = row.takeWhile { it != null }.filterNotNull()
            assertEquals(2, modifiers.first { it.latch == Latch.Shift }.span, "$name draws shift one column wide")
            assertTrue(
                modifiers.none { it.kind == CapKind.Blank },
                "$name still spends a slot of the modifier group on a blank",
            )
        }
    }

    // Every layer draws the same number of columns, which is what lets a thumb keep a key across
    // one. A cap spanning two is two of them.
    @Test
    fun everyRowOfEveryLayoutIsTheSameNumberOfColumnsWide() {
        for ((name, rows) in namedLayouts()) {
            val widths = rows.map { row -> row.filterNotNull().sumOf { it.span } }
            assertEquals(1, widths.distinct().size, "$name draws rows of different widths: $widths")
        }
    }

    @Test
    fun eachRowIsSplitByExactlyOneSeparator() {
        for (rows in listOf(KeyLayouts.portrait, KeyLayouts.portraitFn, KeyLayouts.landscape, KeyLayouts.landscapeFn)) {
            for (row in rows) assertEquals(1, row.count { it == null })
        }
    }

    // Per layout rather than across both of them. The union passed while a phone in portrait could
    // The operator: *"when I press fn to get out of fn I need to press fn twice"*.
    //
    // **`fn` is a layer and not a prefix.** It has no armed state to be in: `consume` deliberately
    // leaves it standing where it clears the other three, and the row reads nothing but
    // `active()` — so `Armed` and `Locked` draw the same cap over the same layer, and the
    // three-state cycle spent a whole press moving between two states nothing can tell apart. The
    // way out of a layer is the press that turned it on.
    @Test
    fun oneMorePressOfFnLeavesTheLayerItTurnedOn() {
        val latches = Latches()
        latches.tap(Latch.Fn)
        assertTrue(latches.fn.active(), "the first press did not turn the layer on")
        latches.tap(Latch.Fn)
        assertFalse(latches.fn.active(), "the layer took two presses to leave")
    }

    // And the other three keep the cycle, which is what a prefix wants: one key, a run of them,
    // off again.
    @Test
    fun aPrefixStillArmsThenLocksThenClears() {
        for (latch in listOf(Latch.Ctrl, Latch.Alt, Latch.Shift)) {
            val latches = Latches()
            latches.tap(latch)
            assertEquals(LatchState.Armed, latches[latch], "$latch did not arm")
            latches.tap(latch)
            assertEquals(LatchState.Locked, latches[latch], "$latch did not lock")
            latches.tap(latch)
            assertEquals(LatchState.Off, latches[latch], "$latch did not clear")
        }
    }

    // The operator, on 0.1.66: *"we have a `fn` button but it only gives me F1-F6"*.
    //
    // **A key behind a long press is a key nobody has.** F7 to F12 were on the layer, as the
    // alternate of the cap six along from them, and nothing on the row said so — which is the
    // same defect the fn layer itself was built to fix one release earlier, one level down. The
    // reading that matters to a thumb is what it can see: every function key has a cap.
    @Test
    fun everyFunctionKeyHasACapOfItsOwnOnTheFnLayer() {
        for ((name, rows) in listOf("portrait" to KeyLayouts.portraitFn, "landscape" to KeyLayouts.landscapeFn)) {
            val caps = rows.flatten().filterNotNull()
            for (n in 1..12) {
                assertTrue(
                    caps.any { it.label == "F$n" },
                    "F$n is reachable on the $name Fn layer only by a gesture nothing names",
                )
            }
        }
    }

    // The other half of the same report: *"probably also needs to not replace existing buttons so
    // I could alt+f4 for example"*.
    //
    // A layer that swallows the modifiers cannot build a chord on itself. The latches do survive
    // the switch — arming alt and then turning the layer on has always worked — but that is a
    // sequence nobody can see from a row with no `alt` on it, and every other layer here is judged
    // by what it draws rather than by what it will accept.
    //
    // Shift is the third of them, and it was the one missing: *"there's also no way to toggle
    // shift with fn keys up?"*. It takes the slot `kbd` had, on the operator's own reading
    // (*"maybe replace kbd with shift in FN mode?"*) — see `everyLayoutThatIsNotALayerReachesTheKeyboardToggle`.
    @Test
    fun theFnLayerKeepsTheModifiersAChordIsBuiltFrom() {
        for ((name, rows) in listOf("portrait" to KeyLayouts.portraitFn, "landscape" to KeyLayouts.landscapeFn)) {
            val latches = rows.flatten().filterNotNull().mapNotNull { it.latch }
            for (modifier in listOf(Latch.Ctrl, Latch.Alt, Latch.Shift)) {
                assertTrue(modifier in latches, "the $name Fn layer has no $modifier to chord with")
            }
        }
    }

    // And the chord itself, built where the operator would build it: both presses on the layer,
    // one modified key out. Each of the three, because drawing a modifier on a layer and having it
    // reach the key beside it are two different claims.
    @Test
    fun aModifierAndAFunctionKeyPressedOnTheFnLayerAreOneChord() {
        for ((name, rows) in listOf("portrait" to KeyLayouts.portraitFn, "landscape" to KeyLayouts.landscapeFn)) {
            for (latch in listOf(Latch.Ctrl, Latch.Alt, Latch.Shift)) {
                // One set of latches, the way the pane builds them: the row arms them and the sink
                // reads them, and a test that gave each its own would prove nothing about a chord.
                val recorder = Recorder()
                val session = PaneSession("n/w1:p1")
                val keys = InputSink("n/w1:p1", recorder, session.latches)
                val caps = rows.flatten().filterNotNull()
                capPress(caps.first { it.latch == latch }, session, keys)
                capPress(caps.first { it.label == "F4" }, session, keys)
                assertEquals(
                    listOf(
                        Esc.modified(
                            Esc.function(4),
                            ctrl = latch == Latch.Ctrl,
                            alt = latch == Latch.Alt,
                            shift = latch == Latch.Shift,
                        ),
                    ),
                    recorder.text,
                    "$name: $latch and F4 did not leave the row as one chord",
                )
            }
        }
    }

    // The operator, on 0.1.57: *"function keys (F1-F12) can we add a way to show these, maybe
    // replace `-` with `fn` and it shows the options?"* — they were already there, behind a long
    // press on `alt` that nothing on the row named, in nothing the row drew, and in no label a
    // screen reader could read out. A layer with no key of its own is a layer nobody finds.
    @Test
    fun theFnLayerHasAKeyOfItsOwnOnEveryLayout() {
        for ((name, rows) in namedLayouts()) {
            assertTrue(
                rows.flatten().filterNotNull().any { it.latch == Latch.Fn },
                "$name reaches the function keys only by a gesture nothing names",
            )
        }
    }

    // And pressing it twice is two presses in one place. The cap that turns the layer on has to be
    // in the slot the cap that turns it off is in, or the way back is somewhere the thumb has to
    // go looking for.
    @Test
    fun theFnKeyDoesNotMoveWhenItIsPressed() {
        for ((name, pair) in listOf(
            "portrait" to (KeyLayouts.portrait to KeyLayouts.portraitFn),
            "landscape" to (KeyLayouts.landscape to KeyLayouts.landscapeFn),
        )) {
            assertEquals(fnSlot(pair.first), fnSlot(pair.second), "$name moves its fn key")
        }
    }

    // The operator: *"when I use / or | on the virtual keyboard it doesn't end up in the keyboard
    // history like the real one"*. A cap writes straight to the pane, past the buffer the soft
    // keyboard reads its suggestions and corrections from, so the field lets go of the line it was
    // mirroring — and a symbol the keyboard already has cost the operator the word in front of it.
    // A cap earns its slot by being a key the soft keyboard does not have.
    @Test
    fun noCapTypesACharacterTheSoftKeyboardAlreadyHas() {
        for (cap in allCaps().filter { it.kind == CapKind.Text }) {
            val printable = cap.send.length == 1 && cap.send[0].code >= 0x20 && cap.send[0] != '\u007f'
            assertFalse(printable, "${cap.label} types a character the soft keyboard already has")
        }
    }

    // The slot `/` had is where shift went. It rides the arrows, tab and home/end already on this
    // row, and it was reachable only as a long press on ctrl that nothing on the row named.
    @Test
    fun shiftHasACapOfItsOwnOnTheLayoutsThatCarryTheArrows() {
        for ((name, rows) in listOf("portrait" to KeyLayouts.portrait, "landscape" to KeyLayouts.landscape)) {
            assertTrue(
                rows.flatten().filterNotNull().any { it.kind == CapKind.Latch && it.latch == Latch.Shift },
                "$name reaches shift only by a gesture nothing names",
            )
        }
    }

    // The keyboard toggle stands on the layouts that are not a layer. It used to stand on all
    // four, on the reading that with fn on and no `kbd` the soft keyboard could not be brought
    // back without leaving the layer first — which was written while leaving the layer took two
    // presses in a place that looked like one. It takes one now
    // (`oneMorePressOfFnLeavesTheLayerItTurnedOn`), the layer has no slot that is not spoken for,
    // and a chord needs shift more than a layer of function keys needs the keyboard beside it:
    // `ctrl` and `alt` on the fn layer ask for the keyboard themselves when they are armed.
    @Test
    fun everyLayoutThatIsNotALayerReachesTheKeyboardToggle() {
        for ((name, rows) in listOf("portrait" to KeyLayouts.portrait, "landscape" to KeyLayouts.landscape)) {
            assertTrue(
                rows.flatten().filterNotNull().any { it.kind == CapKind.Keyboard },
                "$name has no keyboard key",
            )
        }
    }
}

private fun namedLayouts() = listOf(
    "portrait" to KeyLayouts.portrait,
    "portrait fn" to KeyLayouts.portraitFn,
    "landscape" to KeyLayouts.landscape,
    "landscape fn" to KeyLayouts.landscapeFn,
)

// Which column a slot starts in, counting what the caps before it span rather than how many of
// them there are.
private fun columnOf(row: List<dev.kampr.terminal.input.KeyCap?>, slot: Int): Int =
    row.take(slot).filterNotNull().sumOf { it.span }

// Row and column, because a cap two columns wide is one slot and two of what a thumb sees: the
// `fn` cap is in the same *place* on both portrait layouts and in a different slot of the list.
private fun fnSlot(rows: List<List<dev.kampr.terminal.input.KeyCap?>>): Pair<Int, Int> {
    for ((r, row) in rows.withIndex()) {
        val at = row.indexOfFirst { it?.latch == Latch.Fn }
        if (at >= 0) return r to columnOf(row, at)
    }
    return -1 to -1
}

// A pane whose program holds the alternate screen keeps no ring (#387), so the scroll it cannot
// give is handed to the program instead — by the notch from a wheel, by the row from a finger, and
// in the dialect that program understands.
class PaneScrollTest {
    private fun reports(keys: ScrollKeys, up: Boolean): List<String> {
        val sent = mutableListOf<String>()
        PaneScroll(keys) { sent += it }.wheel(rows = if (up) 3 else -3, col = 40, row = 20)
        return sent
    }

    @Test
    fun aHarnessThatAskedForTheMouseGetsAWheelReportARowAndNothingElseMoves() {
        assertEquals(List(3) { "\u001b[<64;41;21M" }, reports(ScrollKeys.Wheel, up = true))
        assertEquals(List(3) { "\u001b[<65;41;21M" }, reports(ScrollKeys.Wheel, up = false))
    }

    // Alternate scroll, which is what herdr does at the desk. The **application** form: `less`,
    // `man` and `vim` all set DECCKM, and the normal `ESC [ B` moved less by nothing (#390).
    @Test
    fun everythingElseGetsApplicationCursorKeysAThreeRowNotchAtATime() {
        assertEquals(List(3) { "\u001bOA" }, reports(ScrollKeys.CursorKeys, up = true))
        assertEquals(List(3) { "\u001bOB" }, reports(ScrollKeys.CursorKeys, up = false))
    }

    // The gate, and it fails closed. A null `cmd` is a pane at its prompt *or* a pane nothing could
    // read (#297) — and cursor keys into a shell's line editor recall its history. A harness label
    // outlives the harness, so `agent` alone is not enough to send anything on.
    @Test
    fun aPaneWhoseForegroundJobIsUnknownIsNeverTypedInto() {
        assertEquals(null, paneScrollKeys(agent = null, cmd = null))
        assertEquals(null, paneScrollKeys(agent = "claude", cmd = null), "a stale label typed at a prompt")
    }

    @Test
    fun aMeasuredHarnessIsUpgradedAndEverythingElseTakesTheDefault() {
        assertEquals(ScrollKeys.Wheel, paneScrollKeys(agent = "claude", cmd = "claude"))
        assertEquals(ScrollKeys.CursorKeys, paneScrollKeys(agent = "codex", cmd = "codex"))
        assertEquals(ScrollKeys.CursorKeys, paneScrollKeys(agent = null, cmd = "less"))
        assertEquals(ScrollKeys.CursorKeys, paneScrollKeys(agent = null, cmd = "vim"))
    }

    // A row of travel asks for a row, and what is left over is kept: rounding each frame's few
    // pixels to nothing is a drag that moves the finger and never the pane. The rows are queued
    // rather than sent (#527), so the accounting is read off the queue and then drained.
    @Test
    fun aRefusedDragAsksForARowPerRowAndCarriesTheRemainder() {
        val sent = mutableListOf<String>()
        val scroll = PaneScroll(ScrollKeys.CursorKeys) { sent += it }
        repeat(4) { scroll.refused(30f, step = 100f, col = 0, row = 0) }
        assertEquals(1, scroll.queued, "120px of travel across four frames asked for one row")
        scroll.refused(-260f, step = 100f, col = 0, row = 0)
        // A row queued and not yet sent, against two rows the other way, is one row the other way:
        // reversing inside a gesture costs the program nothing rather than two wasted round trips.
        assertEquals(-1, scroll.queued, "the drag turned round and the rows did not net off")
        while (scroll.drain()) Unit
        assertEquals(listOf("\u001bOB"), sent, "back up the screen is a scroll down")
    }

    // What a batch of reports is worth to Claude, measured on a real pane (#567): reports that
    // land together ramp, and the first one after a reversal moves nothing. Each row is travel
    // asked for and the one write it has to go out as — the finger's travel, not a report per row.
    @Test
    fun aDragsWholeTravelGoesOutAsOneWriteWorthExactlyThatMuchToClaude() {
        val table = listOf(
            Triple(1, false, 1),
            Triple(3, false, 3),
            Triple(6, false, 5),
            Triple(19, false, 10),
            Triple(67, false, 20),
            Triple(1, true, 2),
            Triple(16, true, 10),
            Triple(61, true, 20),
        )
        for ((rows, reversed, reports) in table) {
            val sent = mutableListOf<String>()
            val scroll = PaneScroll(ScrollKeys.Wheel) { sent += it }
            if (reversed) {
                scroll.refused(-100f, step = 100f, col = 0, row = 0)
                scroll.drain()
                sent.clear()
            }
            repeat(rows) { scroll.refused(100f, step = 100f, col = 0, row = 0) }
            assertTrue(scroll.drain().not(), "$rows rows left travel behind: ${scroll.queued}")
            assertEquals(1, sent.size, "$rows rows went out as ${sent.size} writes")
            assertEquals(
                "\u001b[<64;1;1M".repeat(reports),
                sent.single(),
                "$rows rows${if (reversed) " after a reversal" else ""} is $reports reports to claude",
            )
        }
    }

    // Travel a batch cannot land exactly is carried rather than overshot: 7 rows is 5 reports
    // landing 6, because a sixth would land 8.
    @Test
    fun whatABatchCannotLandExactlyWaitsForTheNextOne() {
        val sent = mutableListOf<String>()
        val scroll = PaneScroll(ScrollKeys.Wheel) { sent += it }
        repeat(7) { scroll.refused(100f, step = 100f, col = 0, row = 0) }
        assertTrue(scroll.drain(), "the batch claimed to land all 7 rows")
        assertEquals("\u001b[<64;1;1M".repeat(5), sent.single(), "7 rows is 5 reports landing 6")
        assertEquals(1, scroll.queued)
    }

    // #567's table, measured on a real Claude one write at a time: fresh, and after a reversal —
    // whose first report moves nothing whatever the ramp stood at.
    @Test
    fun oneWriteLandsWhatClaudeMeasuredForIt() {
        val counts = listOf(1, 2, 3, 4, 5, 8, 10, 15, 20, 25, 30, 40)
        val fresh = listOf(1, 2, 3, 4, 6, 13, 19, 39, 67, 97, 127, 187)
        val reversed = listOf(0, 1, 2, 3, 4, 10, 16, 34, 61, 91, 121, 181)
        for ((i, k) in counts.withIndex()) {
            assertEquals(fresh[i], ScrollKeys.Wheel.land(k, reversed = false).rows, "$k fresh")
            assertEquals(reversed[i], ScrollKeys.Wheel.land(k, reversed = true, ramp = 6.0).rows, "$k reversed")
        }
    }

    // The ramp is one multiplier every report inside 40 ms of the last moves on, whichever write it
    // came in (#569) — #567 measured two writes of 5 landing 19 rows 5-35 ms apart. So two
    // writes carried back to back land exactly what one write of both would, at any split.
    @Test
    fun aWriteThatCarriesTheRampLandsWhatOneWriteOfBothWould() {
        for (first in listOf(1, 3, 5, 10, 17, 25)) {
            for (second in listOf(1, 2, 5, 12, 30)) {
                val a = ScrollKeys.Wheel.land(first, reversed = false)
                val b = ScrollKeys.Wheel.land(second, reversed = false, ramp = a.ramp)
                val whole = ScrollKeys.Wheel.land(first + second, reversed = false)
                assertEquals(whole.rows, a.rows + b.rows, "$first reports then $second")
                assertEquals(whole.ramp, b.ramp, "$first reports then $second left the ramp elsewhere")
            }
        }
        assertEquals(13, ScrollKeys.Wheel.land(5, reversed = false, ramp = ScrollKeys.Wheel.land(5, false).ramp).rows)
    }

    // Sized from where the last write left the ramp: after five reports it stands at 2.2, so 12 rows
    // is 2 + 2 + 3 + 3 with 2 left over. Sized fresh it would be seven reports, which Claude —
    // carrying — would have taken 21 rows for.
    @Test
    fun aCarriedWriteIsSizedFromWhereTheLastLeftTheRamp() {
        val sent = mutableListOf<String>()
        val scroll = PaneScroll(ScrollKeys.Wheel) { sent += it }
        repeat(6) { scroll.refused(100f, step = 100f, col = 0, row = 0) }
        assertFalse(scroll.drain())
        repeat(12) { scroll.refused(100f, step = 100f, col = 0, row = 0) }
        assertTrue(scroll.drain(carry = true))
        assertEquals("\u001b[<64;1;1M".repeat(4), sent[1], "12 rows carried on from 5 reports")
        assertEquals(2, scroll.queued)
    }

    // A carried report is worth at least two rows past the fifth, so a finger that has moved less
    // than that since the last write is not sent anything: one report would overshoot it.
    @Test
    fun aCarriedWriteThatWouldOvershootSendsNothing() {
        val sent = mutableListOf<String>()
        val scroll = PaneScroll(ScrollKeys.Wheel) { sent += it }
        repeat(6) { scroll.refused(100f, step = 100f, col = 0, row = 0) }
        scroll.drain()
        scroll.refused(100f, step = 100f, col = 0, row = 0)
        assertTrue(scroll.drain(carry = true), "the row claimed to have gone out")
        assertEquals(1, sent.size, "a carried report worth 2 rows was sent for 1")
        assertFalse(scroll.drain(), "the same row, fresh, is one report")
        assertEquals(2, sent.size)
    }

    // A program that does not ramp is sent a row per row, still in one write.
    @Test
    fun cursorKeysAreARowAPieceInOneWrite() {
        val sent = mutableListOf<String>()
        val scroll = PaneScroll(ScrollKeys.CursorKeys) { sent += it }
        repeat(12) { scroll.refused(-100f, step = 100f, col = 0, row = 0) }
        assertEquals(false, scroll.drain())
        assertEquals(listOf("\u001bOB".repeat(12)), sent)
    }

    // The bound on how far a finger may run ahead of the program: one write's worth. A finger
    // faster than that is a wheel spun faster than the program follows, and the excess is dropped.
    @Test
    fun aFingerMayNotRunMoreThanOneWriteAheadOfTheProgram() {
        val scroll = PaneScroll(ScrollKeys.Wheel) { }
        repeat(4000) { scroll.refused(100f, step = 100f, col = 0, row = 0) }
        assertEquals(187, scroll.queued, "4000 rows of travel queued ${scroll.queued} rows")
    }

    // The wheel is inside the pipeline's capacity already — a hand makes 10-30 detents a second
    // against the 20-35 it returns — so a notch is not queued behind anything (#527).
    @Test
    fun aWheelNotchIsNotPacedBecauseAHandNeverOutrunsTheProgram() {
        val sent = mutableListOf<String>()
        val scroll = PaneScroll(ScrollKeys.Wheel) { sent += it }
        scroll.wheel(rows = 3, col = 0, row = 0)
        assertEquals(3, sent.size, "a notch waited for a pace it does not need")
        assertEquals(0, scroll.queued)
    }

    // The instrument, not the behaviour. A gesture that reaches a program is the one thing this
    // client cannot see the cost of from outside — `dumpsys gfxinfo` counts frames the app drew,
    // which on a live pane swung between 0% and 12.5% jank on identical idle windows — so the
    // count has to be taken where the reports are actually sent, and it has to be right.
    @Test
    fun theTraceCountsEveryReportAGesturePutOnTheWire() {
        val lines = mutableListOf<String>()
        val trace = ScrollTrace(on = true) { lines += it }
        val scroll = PaneScroll(ScrollKeys.Wheel, trace) { }
        repeat(5) { scroll.refused(100f, step = 100f, col = 0, row = 0) }
        while (scroll.drain()) Unit
        scroll.wheel(rows = 3, col = 0, row = 0)
        trace.arrived()
        scroll.rest()
        assertEquals(1, lines.size, "one gesture is one line")
        assertTrue(
            lines[0].contains("reports=8"),
            "five rows of drag and a three-row notch is eight reports on the wire, not ${lines[0]}",
        )
        assertTrue(lines[0].contains("frames=1"), "the frame that answered them was not counted")
    }

    // Frames a second is the smoothness a finger sees: #568's pump returned ~24-38 of them on a
    // real Claude and the carried pump ~41-53 (#571). Taken over the gesture's own span —
    // first write to last frame — not over the idle gap that ends it, or it reads as a stall.
    @Test
    fun theTraceSaysHowManyFramesASecondTheGestureGotAndHowManyWritesCarried() {
        val clock = object : kotlin.time.TimeSource {
            var now = 0L
            override fun markNow(): kotlin.time.TimeMark = object : kotlin.time.TimeMark {
                val at = now
                override fun elapsedNow() = (now - at).milliseconds
            }
        }
        val lines = mutableListOf<String>()
        val trace = ScrollTrace(on = true, emit = { lines += it }, clock = clock)
        trace.sent(ScrollKeys.Wheel, reports = 4, rows = 4)
        repeat(10) {
            clock.now += 10
            trace.arrived()
            clock.now += 10
            trace.sent(ScrollKeys.Wheel, reports = 1, rows = 2, carried = true)
        }
        clock.now += 10
        trace.arrived()
        clock.now += 2_000
        trace.flush(ScrollKeys.Wheel)
        val line = lines.single()
        assertTrue(line.contains("frames=11"), line)
        assertTrue(line.contains("carried=10"), line)
        assertTrue(line.contains("ms=210 "), "the span ran past the gesture's last frame: $line")
        assertTrue(line.contains("fps=52"), line)
    }

    // A pane nobody asked to trace pays nothing and says nothing.
    @Test
    fun anUntracedPaneEmitsNothingAtAll() {
        val lines = mutableListOf<String>()
        val scroll = PaneScroll(ScrollKeys.Wheel, ScrollTrace(on = false) { lines += it }) { }
        repeat(5) { scroll.refused(100f, step = 100f, col = 0, row = 0) }
        scroll.rest()
        assertEquals(emptyList(), lines)
    }

    // Leftovers belong to the gesture that made them. Carried across, the first row of a fresh
    // drag arrives before the finger has travelled it.
    @Test
    fun aGesturesLeftoversDoNotArriveInTheNextOne() {
        val sent = mutableListOf<String>()
        val scroll = PaneScroll(ScrollKeys.CursorKeys) { sent += it }
        scroll.refused(90f, step = 100f, col = 0, row = 0)
        assertEquals(0, scroll.queued, "90 of a 100px row was already a row")
        scroll.rest()
        scroll.refused(90f, step = 100f, col = 0, row = 0)
        assertEquals(0, scroll.queued, "the last drag's leftovers arrived in this one")
        while (scroll.drain()) Unit
        assertEquals(emptyList(), sent)
    }

    // The whole table, because the defect was one row of it: `ctrl+shift+C` lowercased to `c` and
    // went to the pane as `^C`, so copying interrupted the process, and `⌘C` did the same. C and V
    // are taken only with shift or the command key, K only without shift — everything else is
    // still a terminal's own control byte.
    @Test
    fun onlyTheCopyPasteAndPaletteChordsAreTakenOffThePane() {
        val table = listOf(
            Triple('c', "ctrl", null),
            Triple('v', "ctrl", null),
            Triple('c', "ctrl+shift", PaneChord.Copy),
            Triple('C', "ctrl+shift", PaneChord.Copy),
            Triple('v', "ctrl+shift", PaneChord.Paste),
            Triple('V', "ctrl+shift", PaneChord.Paste),
            Triple('c', "meta", PaneChord.Copy),
            Triple('v', "meta", PaneChord.Paste),
            Triple('a', "ctrl+shift", null),
            Triple('a', "ctrl", null),
            Triple('t', "meta", null),
            Triple('c', "", null),
            Triple('c', "shift", null),
            Triple('k', "ctrl", PaneChord.Palette),
            Triple('K', "ctrl", PaneChord.Palette),
            Triple('k', "meta", PaneChord.Palette),
            Triple('k', "ctrl+shift", null),
            Triple('k', "", null),
        )
        for ((key, mods, wanted) in table) {
            val got = paneChord(
                key,
                ctrl = mods.contains("ctrl"),
                meta = mods.contains("meta"),
                shift = mods.contains("shift"),
            )
            assertEquals(wanted, got, "$mods+$key")
        }
    }

    // A chord carrying the platform's command key is the platform's: `⌘T`, `⌘W` and `⌘L` are the
    // browser's, and turning one into `^T` both interrupted the pane and stole the new tab.
    @Test
    fun aCommandChordIsNeverAControlByte() {
        assertTrue(chordSendsControl(ctrl = true, meta = false), "ctrl stopped making a control byte")
        assertFalse(chordSendsControl(ctrl = true, meta = true), "a command chord made a control byte")
        assertFalse(chordSendsControl(ctrl = false, meta = true), "a command chord made a control byte")
        assertFalse(chordSendsControl(ctrl = false, meta = false))
        assertNull(paneChord('x', ctrl = false, meta = true, shift = false))
    }
}

// The keystroke half of the same instrument. The only latency the app showed was the node's ping to
// herdr, which is one leg of a keystroke's trip and says nothing about the rest — so a keystroke
// that visibly stalls behind a 2 ms figure has to be taken apart where it is typed: the wait for
// the frame that answers it, and the wait from that frame to the one the client paints.
class KeyTraceTest {
    private class Clock : kotlin.time.TimeSource {
        var now = 0L
        override fun markNow(): kotlin.time.TimeMark = object : kotlin.time.TimeMark {
            val at = now
            override fun elapsedNow() = (now - at).milliseconds
        }
    }

    private class Rig(on: Boolean = true) {
        val clock = Clock()
        val lines = mutableListOf<String>()
        val echoes = mutableListOf<Long>()
        val trace = KeyTrace(on = on, emit = { lines += it }, clock = clock, echoed = { echoes += it })
        var col = 0

        fun type(answerIn: Long) {
            trace.sent()
            clock.now += answerIn
            col++
            trace.frame(col, 0)
            clock.now += 5L
            trace.drawn()
            clock.now += 100L
        }
    }

    @Test
    fun aBurstOfTypingIsSplitIntoTheRoundTripAndThePaint() {
        val rig = Rig()
        rig.trace.frame(0, 0)
        rig.type(20L)
        rig.type(20L)
        rig.type(400L)
        rig.type(30L)
        rig.trace.flush()
        assertEquals(1, rig.lines.size, "one burst is one line: ${rig.lines}")
        val line = rig.lines[0]
        assertTrue(line.startsWith("KAMPR_KEYS "), line)
        assertTrue(line.contains("keys=4"), line)
        assertTrue(line.contains("echo_p50=30ms"), line)
        assertTrue(line.contains("echo_max=400ms"), line)
        assertTrue(line.contains("paint_max=5ms"), line)
        assertTrue(line.contains("stalls=1"), "a 400 ms echo is a stall a hand can feel: $line")
        assertEquals(listOf(20L, 20L, 400L, 30L), rig.echoes, "every answered key is a sample for the herd")
    }

    // A pane that is working repaints whether anybody types or not — Claude's spinner is a frame
    // every tick — so the first frame after a key is not its answer. The caret moving is.
    @Test
    fun aFrameThatLeavesTheCaretWhereItWasIsNotTheAnswer() {
        val rig = Rig()
        rig.trace.frame(4, 2)
        rig.trace.sent()
        rig.clock.now += 3L
        rig.trace.frame(4, 2)
        rig.clock.now += 40L
        rig.trace.frame(5, 2)
        assertEquals(listOf(43L), rig.echoes)
    }

    // A key typed before the last was answered waits on the same frame, and is not timed twice.
    @Test
    fun aKeyTypedAheadOfItsAnswerSharesIt() {
        val rig = Rig()
        rig.trace.frame(0, 0)
        rig.trace.sent()
        rig.clock.now += 10L
        rig.trace.sent()
        rig.clock.now += 20L
        rig.trace.frame(2, 0)
        assertEquals(listOf(30L), rig.echoes)
    }

    // A key that moves nothing — an arrow at the end of a line, a press a dialog swallowed — is
    // never answered, and the next keystroke must not be charged for the wait.
    @Test
    fun aKeyNothingAnsweredIsDroppedRatherThanReadAsAStall() {
        val rig = Rig()
        rig.trace.frame(0, 0)
        rig.trace.sent()
        rig.clock.now += 5_000L
        rig.type(15L)
        assertEquals(listOf(15L), rig.echoes)
    }

    @Test
    fun theHerdIsToldEvenWhenNobodyIsTracing() {
        val rig = Rig(on = false)
        rig.trace.frame(0, 0)
        rig.type(12L)
        rig.trace.flush()
        assertEquals(emptyList(), rig.lines, "the log line is the trace's; nobody asked for it")
        assertEquals(listOf(12L), rig.echoes)
    }
}
