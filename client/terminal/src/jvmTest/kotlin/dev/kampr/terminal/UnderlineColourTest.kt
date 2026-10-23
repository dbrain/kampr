package dev.kampr.terminal

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.runDesktopComposeUiTest
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.wire.ColorSpec
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Style
import kotlin.test.Test
import kotlin.test.assertTrue

// SGR 58: a spell checker's red squiggle, a linter's yellow one. The line is drawn in its own
// colour; the text under it keeps the text's. Read off the pixels, because the grid paints into a
// `Canvas` nothing in the semantics tree can see into.
@OptIn(ExperimentalTestApi::class)
class UnderlineColourTest {
    private fun pane(ul: ColorSpec): PaneState {
        val styles = StyleTable()
        styles.append(1, listOf(Style(underline = true, ul = ul)))
        val pane = PaneState(Phone.PANE, styles)
        pane.applyReset(
            ServerMsg.GridReset(
                pane = Phone.PANE,
                cols = 94,
                rows = 40,
                rowsData = listOf(RowDiff(0, listOf(Run(0, "$ "), Run(1, "            ")))),
                cursor = Cursor(0, 1, false),
                links = emptyList(),
            ),
        )
        return pane
    }

    private fun redPixels(ul: ColorSpec): Int {
        var count = 0
        runDesktopComposeUiTest(411, 914) {
            val session = PaneSession(Phone.PANE)
            phoneTerminal(pane(ul), session)
            val grid = session.grid
            val map = onRoot().captureToImage().toPixelMap()
            val left = (grid.originX + 2 * grid.cellWidth).toInt()
            val right = (grid.originX + 14 * grid.cellWidth).toInt()
            val top = grid.originY.toInt()
            val bottom = (grid.originY + grid.cellHeight).toInt()
            for (y in top until bottom) {
                for (x in left until right) {
                    val c = map[x, y]
                    if (c.red > 0.8f && c.green < 0.2f && c.blue < 0.2f) count++
                }
            }
        }
        return count
    }

    @Test
    fun anUnderlineIsDrawnInItsOwnColour() {
        val red = redPixels(ColorSpec.Rgb(255, 0, 0))
        assertTrue(red > 0, "no line in the underline's colour was drawn")
        assertTrue(redPixels(ColorSpec.Default) == 0, "a plain underline came out red")
    }
}
