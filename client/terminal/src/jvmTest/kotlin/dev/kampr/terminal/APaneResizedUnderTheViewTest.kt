package dev.kampr.terminal

import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.view.CARET_SETTLE_MS
import kotlin.test.Test
import kotlin.test.assertEquals

// The same defect as the ring's, on the live grid's own shape. `TerminalView` reads
// `pane.cells.cols` in its composition body — the surface's width, the pan clamp, the cell the
// pointer is over, the number the column strip prints — and `CellBuffer.cols` was a plain `var`,
// so a `grid.reset` that reflowed the pane never told the composition it had.
//
// **Rows were covered by accident and columns by nothing.** The only snapshot state that body
// reads about the pane's shape is `pane.cursor`, and the only other thing that moves with a
// reshape is `settledContent`, which is `liveRows` minus the row the record ends on — so a change
// in the *row* count always shows up there and a change in the *column* count never does. Dragging
// a desk window wider is exactly that reshape: the rows stay, the columns move, and ADR 0013's
// standing hold claims the pane at the new width a quarter-second later, by which time nothing
// else on this surface is changing.
private val DESK = 1600.dp to 900.dp

private const val ROWS = 33
private const val WAS = 94
private const val NOW = 120

private const val PROMPT = "[20:36:31 dbrain@comingclean kampr]$ "

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.reflowed(pane: PaneState, cols: Int) {
    pane.applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = cols,
            rows = ROWS,
            rowsData = listOf(RowDiff(0, listOf(Run(0, PROMPT)))),
            // Where a reflow leaves the caret on a shell that has printed one line and been made
            // wider: the same row, the same column. Nothing in this message says the shape twice.
            cursor = Cursor(PROMPT.length, 0, true),
            links = emptyList(),
        ),
    )
    mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
class APaneResizedUnderTheViewTest {
    @Test
    fun a_pane_made_wider_without_its_caret_moving_is_painted_at_the_width_it_now_has() =
        runComposeUiTest {
            val pane = PaneState(Phone.PANE, StyleTable())
            val session = PaneSession(Phone.PANE)
            reflowed(pane, WAS)
            phoneTerminal(pane, session, width = DESK.first, height = DESK.second)
            mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
            waitForIdle()
            assertEquals(WAS, session.grid.cols, "the pane has to open at its own width")

            reflowed(pane, NOW)

            assertEquals(
                NOW,
                session.grid.cols,
                "the pane is $NOW columns and the surface is still laid out as $WAS — the pan " +
                    "stops ${NOW - WAS} columns short of the right edge, the strip prints the old " +
                    "count, and a click reports the wrong cell",
            )
        }
}
