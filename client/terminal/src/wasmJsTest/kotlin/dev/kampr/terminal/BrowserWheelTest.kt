package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import kotlin.math.floor
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private class ClaudeIo : PaneIo {
    val typed = mutableListOf<String>()
    override fun send(msg: ClientMsg) {
        if (msg is ClientMsg.InputText) typed += msg.text
    }
    override fun prefs(paneId: String) = PanePrefs()
    override fun info(paneId: String) =
        PaneInfo(paneId, "node", agent = "claude", cmd = "claude", cols = 94, rows = 60)
}

// What Chrome hands a page for one notch of a mouse wheel, and CMP's web backend passes the DOM's
// `deltaY` through as `scrollDelta` untouched; the conversation's list moves 1 dp for each.
internal const val CHROME_NOTCH = 100f

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.notchIntoHistory() {
    onRoot().performMouseInput {
        moveTo(Offset(width / 2f, height / 2f))
        scroll(-CHROME_NOTCH)
    }
    frames(2)
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.conversationRows(cellHeight: Float): Int =
    floor(CHROME_NOTCH * density.density / cellHeight).toInt()

// The report, on 0.1.99: "scroll in terminal on desktop wasm ... scrolls much slower than the
// conversation pane". One notch moved the terminal three rows and a Claude pane one report —
// 0.83 of Claude's rows (#527) — where the conversation beside it moved a hundred dp.
@OptIn(ExperimentalTestApi::class)
class BrowserWheelTest {
    @Test
    fun aNotchOverKamprsOwnHistoryMovesAsFarAsTheConversationWould() = runComposeUiTest {
        val session = PaneSession(BROWSER_PANE)
        browserTerminal(shellPane(rows = 200, caretRow = 120), session, DESK)
        val cell = session.grid.cellHeight
        val rows = conversationRows(cell)
        assertTrue(rows > 3, "the notch has to be worth more than the old three rows, or nothing is tested: $rows")

        val before = session.view.scrollY
        notchIntoHistory()
        assertEquals(rows * cell, session.view.scrollY - before, 0.01f, "one notch at a ${cell}px cell")
    }

    @Test
    fun aNotchOverAClaudePaneAsksItForAsManyRowsAsTheConversationWouldMove() = runComposeUiTest {
        val io = ClaudeIo()
        val session = PaneSession(BROWSER_PANE)
        browserTerminal(writtenPane(rows = 60, caretRow = 59), session, DESK, io)
        val rows = conversationRows(session.grid.cellHeight)
        assertTrue(rows > 3, "the notch has to be worth more than one report, or nothing is tested: $rows")
        while (session.view.scrollY < session.view.maxScroll) notchIntoHistory()
        io.typed.clear()

        notchIntoHistory()
        assertTrue(
            io.typed.size in rows..rows + 1,
            "one report per row of the conversation's travel, $rows and the fraction carried: ${io.typed}",
        )
    }
}
