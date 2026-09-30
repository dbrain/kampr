package dev.kampr.terminal

import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private class TimedIo : PaneIo {
    val typed = mutableListOf<String>()
    val echoes = mutableListOf<String>()
    override fun send(msg: ClientMsg) {
        if (msg is ClientMsg.InputText) typed += msg.text
    }
    override fun prefs(paneId: String) = PanePrefs()
    override fun echoed(paneId: String, ms: Long) {
        echoes += paneId
    }
}

// The herd's latency figure is what a keystroke costs, taken where it is typed: a key into the real
// terminal, and the frame that moves the caret, is one sample for the machine the pane is on — and
// a frame that repaints without moving it is not the answer.
@OptIn(ExperimentalTestApi::class)
class TypingIsTimedTest {
    @Test
    fun aKeyAndTheCaretItMovedAreOneSampleForThePane() = runComposeUiTest {
        val io = TimedIo()
        val pane = Phone.shell()
        phoneTerminal(pane, PaneSession(Phone.PANE), io = io)
        val caret = pane.cursor

        onNode(hasSetTextAction()).performTextInput("a")
        waitForIdle()
        assertTrue(io.typed.isNotEmpty(), "the key never left, so nothing here was timed")

        pane.applyPatch(ServerMsg.GridPatch(Phone.PANE, emptyList(), Cursor(caret.col, caret.row, true), emptyList()))
        waitForIdle()
        assertEquals(emptyList(), io.echoes, "a repaint that left the caret alone answered the key")

        pane.applyPatch(ServerMsg.GridPatch(Phone.PANE, emptyList(), Cursor(caret.col + 1, caret.row, true), emptyList()))
        waitForIdle()
        assertEquals(listOf(Phone.PANE), io.echoes)
    }
}
