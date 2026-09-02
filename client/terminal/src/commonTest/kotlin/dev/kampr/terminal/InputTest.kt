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
import dev.kampr.terminal.input.PaneScroll
import dev.kampr.terminal.input.ScrollKeys
import dev.kampr.terminal.input.PaneChord
import dev.kampr.terminal.input.chordSendsControl
import dev.kampr.terminal.input.paneChord
import dev.kampr.terminal.input.paneScrollKeys
import kotlin.test.Test
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
    .flatMap { listOfNotNull(it, it.alternate) }

class InputTest {
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
    fun aClosedKeyboardStaysClosedWhenACharacterIsSent() {
        val (_, keys) = sink()
        val session = PaneSession("n/w1:p1")
        val slash = allCaps().first { it.label == "/" }
        capPress(slash, session, keys)
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

        for (cap in listOf(ctrl, alt)) {
            val session = PaneSession("n/w1:p1")
            capHold(cap, session, keys)
            assertFalse(session.keyboardOpen, "holding ${cap.label} rides shift or fn, not letters")
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
            assertEquals(up, down, "up must sit directly above down")
            assertEquals("←", bottom[down - 1]?.label)
            assertEquals("→", bottom[down + 1]?.label)
        }
    }

    @Test
    fun eachRowIsSplitByExactlyOneSeparator() {
        for (rows in listOf(KeyLayouts.portrait, KeyLayouts.portraitFn, KeyLayouts.landscape, KeyLayouts.landscapeFn)) {
            for (row in rows) assertEquals(1, row.count { it == null })
        }
    }

    @Test
    fun theFnLayerReachesEveryFunctionKey() {
        val caps = (KeyLayouts.portraitFn + KeyLayouts.landscapeFn).flatten().filterNotNull()
            .flatMap { listOfNotNull(it, it.alternate) }
        for (n in 1..12) {
            assertTrue(caps.any { it.label == "F$n" }, "F$n is not on the Fn layer")
        }
    }
}

// A pane whose program holds the alternate screen keeps no ring (#387), so the scroll it cannot
// give is handed to the program instead — by the notch from a wheel, by the row from a finger, and
// in the dialect that program understands.
class PaneScrollTest {
    private fun reports(keys: ScrollKeys, up: Boolean): List<String> {
        val sent = mutableListOf<String>()
        PaneScroll(keys) { sent += it }.notch(up = up, col = 40, row = 20)
        return sent
    }

    @Test
    fun aHarnessThatAskedForTheMouseGetsAWheelReportAndNothingElseMoves() {
        assertEquals(listOf("\u001b[<64;41;21M"), reports(ScrollKeys.Wheel, up = true))
        assertEquals(listOf("\u001b[<65;41;21M"), reports(ScrollKeys.Wheel, up = false))
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
    // pixels to nothing is a drag that moves the finger and never the pane.
    @Test
    fun aRefusedDragAsksForARowPerRowAndCarriesTheRemainder() {
        val sent = mutableListOf<String>()
        val scroll = PaneScroll(ScrollKeys.CursorKeys) { sent += it }
        repeat(4) { scroll.refused(30f, step = 100f, col = 0, row = 0) }
        assertEquals(listOf("\u001bOA"), sent, "120px of travel across four frames asked for ${sent.size} rows")
        scroll.refused(-260f, step = 100f, col = 0, row = 0)
        assertEquals(3, sent.size, "the drag turned round and the other direction was not sent")
        assertTrue(sent.drop(1).all { it == "\u001bOB" }, "back up the screen is a scroll down")
    }

    // Leftovers belong to the gesture that made them. Carried across, the first row of a fresh
    // drag arrives before the finger has travelled it.
    @Test
    fun aGesturesLeftoversDoNotArriveInTheNextOne() {
        val sent = mutableListOf<String>()
        val scroll = PaneScroll(ScrollKeys.CursorKeys) { sent += it }
        scroll.refused(90f, step = 100f, col = 0, row = 0)
        assertEquals(emptyList(), sent, "90 of a 100px row was already a row")
        scroll.rest()
        scroll.refused(90f, step = 100f, col = 0, row = 0)
        assertEquals(emptyList(), sent, "the last drag's leftovers arrived in this one")
    }

    // The whole table, because the defect was one row of it: `ctrl+shift+C` lowercased to `c` and
    // went to the pane as `^C`, so copying interrupted the process, and `⌘C` did the same. Only C
    // and V are taken, and only with shift or the command key — everything else is still a
    // terminal's own control byte.
    @Test
    fun onlyTheCopyAndPasteChordsAreTakenOffThePane() {
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
