package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.ScrollWheel
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.swipe
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.view.CARET_SETTLE_MS
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// The two sizes the report came from. "mobile is similarly zoomed in a lot", so this is not a
// desktop-only rule: what makes the bottom unreachable is a grid taller than the rectangle with the
// caret above the last row of it, and every phone against a herdr pane is that.
private val DESK = 1600.dp to 900.dp
private val PHONE = 411.dp to 914.dp

// Enough notches to walk the whole of a 200-row surface at three rows a notch, twice over.
private const val NOTCHES_TO_THE_END = 200

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.tall(size: Pair<Dp, Dp>): Pair<PaneState, PaneSession> {
    val pane = Phone.shell(rows = 200, caretRow = 3)
    val session = PaneSession(Phone.PANE)
    phoneTerminal(pane, session, width = size.first, height = size.second)
    assertTrue(session.view.maxScroll > 0f, "the grid has to overflow, or nothing is tested")
    assertTrue(
        session.view.band.floor > 0f,
        "the caret has to be holding the surface off the bottom of the grid, or nothing is tested",
    )
    return pane to session
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.wheelToTheBottom() {
    repeat(NOTCHES_TO_THE_END) {
        onRoot().performMouseInput {
            moveTo(Offset(width / 2f, height / 2f))
            scroll(1f, ScrollWheel.Vertical)
        }
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.dragToTheBottom() {
    repeat(20) {
        onRoot().performTouchInput {
            swipe(
                start = Offset(centerX, centerY + 300f),
                end = Offset(centerX, centerY - 300f),
                durationMillis = 300,
            )
        }
        waitForIdle()
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.caretTo(pane: PaneState, row: Int) {
    pane.applyPatch(
        ServerMsg.GridPatch(
            pane = Phone.PANE,
            rows = listOf(RowDiff(row, listOf(Run(0, "$ line $row")))),
            cursor = Cursor(0, row, true),
            links = emptyList(),
        ),
    )
    waitForIdle()
}

// The report, verbatim: "the terminal pane on wasm desktop keeps bouncing around and landing back
// where i last typed instead of the bottom of the screen".
//
// The caret band's floor was wired into the drag's own clamp, and the floor is where a *follower*
// rests — the least scroll that leaves the caret on screen — not a limit on where a hand may go.
// So on any pane taller than the rectangle whose caret sits above the last row, everything below
// the caret's screenful was unreachable: the wheel stopped early, and the surface a hand did win
// was thrown back at the next frame that moved the caret, by `max(scrollY, floor)`.
//
// Both halves are the same rule. The band governs a viewport that is following and nothing else.
@OptIn(ExperimentalTestApi::class)
class ReachingTheBottomOfAPaneTest {
    @Test
    fun aWheelReachesTheBottomOfTheGridOnADesk() = runComposeUiTest {
        val (_, session) = tall(DESK)
        wheelToTheBottom()
        assertEquals(
            0f,
            session.view.scrollY,
            0.01f,
            "the wheel stopped ${session.view.scrollY}px above the bottom of the grid, at the " +
                "caret floor of ${session.view.band.floor}px",
        )
    }

    @Test
    fun aWheelReachesTheBottomOfTheGridOnAPhone() = runComposeUiTest {
        val (_, session) = tall(PHONE)
        wheelToTheBottom()
        assertEquals(0f, session.view.scrollY, 0.01f, "the wheel stopped short of the bottom")
    }

    @Test
    fun aDragReachesTheBottomOfTheGrid() = runComposeUiTest {
        val (_, session) = tall(PHONE)
        dragToTheBottom()
        assertEquals(0f, session.view.scrollY, 0.01f, "the finger stopped short of the bottom")
    }

    // And stays there. A reader who went past the live edge to read the tail of a pane is parked
    // there deliberately; the caret moving is the pane's business and not a reason to move them.
    @Test
    fun aReaderAtTheBottomIsNotDraggedBackToTheCaret() {
        for (size in listOf(DESK, PHONE)) {
            runComposeUiTest {
                val (pane, session) = tall(size)
                wheelToTheBottom()
                assertEquals(0f, session.view.scrollY, 0.01f, "$size: never reached the bottom")
                assertTrue(!session.view.following, "$size: the bottom is not the live edge here")

                for (row in listOf(0, 90, 199, 3, 120)) {
                    caretTo(pane, row)
                    mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
                    waitForIdle()
                    assertEquals(
                        0f,
                        session.view.scrollY,
                        0.01f,
                        "$size: the caret settling on row $row took the reader off the bottom " +
                            "of the grid and back to ${session.view.scrollY}px",
                    )
                }
            }
        }
    }
}
