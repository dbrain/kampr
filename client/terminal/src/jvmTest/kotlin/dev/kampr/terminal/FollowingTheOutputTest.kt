package dev.kampr.terminal

import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertTrue

private const val FIRST = 2
private const val LAST = 9
private val BLOCK = FIRST..LAST

private fun patch(rows: List<RowDiff>, cursorRow: Int) = ServerMsg.GridPatch(
    pane = Phone.PANE,
    rows = rows,
    cursor = Cursor(0, cursorRow, true),
    links = emptyList(),
)

// `docker compose pull` on a pane nobody has scrolled: a command line, then a block of one line
// per service that is redrawn in place — the caret walks to the top of the block, rewrites every
// line of it, and returns to the bottom. Every one of those is a frame the node ships.
private fun pulling(): PaneState {
    val pane = PaneState(Phone.PANE, StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = 94,
            rows = 40,
            rowsData = listOf(
                RowDiff(0, listOf(Run(0, "[20:36:31 dbrain@comingclean deploy]$ docker compose pull"))),
                RowDiff(1, listOf(Run(0, "[+] Pulling 8/8"))),
            ),
            cursor = Cursor(0, FIRST, true),
            links = emptyList(),
        ),
    )
    return pane
}

private fun redraw(percent: Int) = BLOCK.map { row ->
    RowDiff(row, listOf(Run(0, " service-${row - FIRST}  Downloading [===>    ] $percent%")))
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.offScreen(pane: PaneState, session: PaneSession): List<Int> =
    BLOCK.filterNot { onScreen(pane, session, it) }

// The report, verbatim: "when running commands that output some lines and update in place like a
// docker pull && docker up -d the wasm app (and maybe others) scroll up/don't track the output …
// when done sometimes i need to scroll up and back down for it to show me the completed terminal
// text entry".
//
// The surface rested *exactly* on the caret floor, and the floor is a minimum, not a place. So
// every frame that moved the caret moved the surface with it, in both directions and by the whole
// distance — and an in-place redraw moves the caret to the top of its block and back several times
// a second. The block being redrawn is what the operator is reading; the caret's excursion to the
// top of it is a rendering artefact of the command, not somewhere anybody asked to look.
@OptIn(ExperimentalTestApi::class)
class FollowingTheOutputTest {
    @Test
    fun anInPlaceRedrawKeepsTheBlockItIsRedrawingOnScreen() {
        runComposeUiTest {
            val pane = pulling()
            val session = PaneSession(Phone.PANE)
            phoneTerminal(pane, session)
            assertTrue(session.view.maxScroll > 0f, "the grid has to overflow, or nothing is tested")

            pane.applyPatch(patch(redraw(0), cursorRow = LAST + 1))
            waitForIdle()
            assertTrue(
                offScreen(pane, session).isEmpty(),
                "rows ${offScreen(pane, session)} of the block were off screen the moment it was drawn",
            )

            for (percent in listOf(30, 60, 90)) {
                pane.applyPatch(patch(redraw(percent), cursorRow = FIRST))
                waitForIdle()
                assertTrue(
                    offScreen(pane, session).isEmpty(),
                    "$percent%: the caret went to the top of the block and took rows " +
                        "${offScreen(pane, session)} of it off the screen",
                )
                pane.applyPatch(patch(emptyList(), cursorRow = LAST + 1))
                waitForIdle()
                assertTrue(
                    offScreen(pane, session).isEmpty(),
                    "$percent%: the caret came back to the bottom of the block and took rows " +
                        "${offScreen(pane, session)} of it off the screen",
                )
            }
        }
    }

    @Test
    fun aCaretThatLeavesTheBlockAndComesStraightBackDoesNotMoveTheSurface() {
        runComposeUiTest {
            val pane = pulling()
            val session = PaneSession(Phone.PANE)
            phoneTerminal(pane, session)
            pane.applyPatch(patch(redraw(0), cursorRow = LAST + 1))
            waitForIdle()
            val resting = session.view.scrollY

            for (percent in listOf(30, 60, 90)) {
                pane.applyPatch(patch(redraw(percent), cursorRow = FIRST))
                waitForIdle()
                assertTrue(
                    abs(session.view.scrollY - resting) < 0.5f,
                    "$percent%: the caret stepping up to the top of the block moved the surface " +
                        "from $resting to ${session.view.scrollY}",
                )
                pane.applyPatch(patch(emptyList(), cursorRow = LAST + 1))
                waitForIdle()
                assertTrue(
                    abs(session.view.scrollY - resting) < 0.5f,
                    "$percent%: the caret stepping back down moved the surface from $resting " +
                        "to ${session.view.scrollY}",
                )
            }
        }
    }
}
