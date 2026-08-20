package dev.kampr.terminal

import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.input.CapKind
import dev.kampr.terminal.input.Esc
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.KeyLayouts
import dev.kampr.terminal.input.Latch
import dev.kampr.terminal.input.Latches
import kotlin.test.Test
import kotlin.test.assertEquals
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
