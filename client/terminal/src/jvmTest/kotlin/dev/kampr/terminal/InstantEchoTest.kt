package dev.kampr.terminal

import androidx.compose.ui.graphics.PixelMap
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runDesktopComposeUiTest
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// A key typed on a line that has been echoing is drawn at once, in the cell and the face the pane's
// own echo will use, and the echo replaces it rather than landing beside it. Read off the pixels,
// because the grid paints into a `Canvas` that nothing in the semantics tree can see into.
@OptIn(ExperimentalTestApi::class)
class InstantEchoTest {
    private fun ComposeUiTest.type(text: String) {
        onNode(hasSetTextAction()).performTextInput(text)
        waitForIdle()
    }

    // The pane's own echo: the character written where the caret was, and the caret one on.
    private fun ComposeUiTest.echo(pane: PaneState, text: String) {
        val caret = pane.cursor
        val line = pane.cells.rowText(caret.row).trimEnd() + text
        pane.applyPatch(
            ServerMsg.GridPatch(
                Phone.PANE,
                listOf(RowDiff(caret.row, listOf(Run(0, line)))),
                Cursor(caret.col + text.length, caret.row, true),
                emptyList(),
            ),
        )
        waitForIdle()
    }

    private fun cell(session: PaneSession, row: Int, col: Int): IntArray {
        val grid = session.grid
        return intArrayOf(
            (grid.originX + col * grid.cellWidth).toInt() + 1,
            (grid.originY + row * grid.cellHeight).toInt() + 1,
            (grid.originX + (col + 1) * grid.cellWidth).toInt() - 1,
            (grid.originY + (row + 1) * grid.cellHeight).toInt() - 1,
        )
    }

    private fun same(a: PixelMap, b: PixelMap, box: IntArray): Boolean {
        for (y in box[1] until box[3]) for (x in box[0] until box[2]) if (a[x, y] != b[x, y]) return false
        return true
    }

    @Test
    fun aKeyOnALineThatEchoesIsDrawnBeforeItsAnswerAndOnlyOnceAfter() = runDesktopComposeUiTest(411, 914) {
        val pane = Phone.shell()
        val session = PaneSession(Phone.PANE)
        phoneTerminal(pane, session)
        type("a")
        echo(pane, "a")
        type("b")
        echo(pane, "b")

        val at = pane.cursor
        val box = cell(session, at.row, at.col)
        val before = onRoot().captureToImage().toPixelMap()
        type("x")
        val guessed = onRoot().captureToImage().toPixelMap()
        assertTrue(!same(before, guessed, box), "the key was typed and its cell did not change until the pane answered")

        echo(pane, "x")
        assertEquals(0, session.echo.shown, "the answer left the guess standing beside it")
        val answered = onRoot().captureToImage().toPixelMap()
        assertTrue(same(guessed, answered, box), "the guess was not drawn the way the pane's own echo is")
    }
}
