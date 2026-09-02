package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.ScrollWheel
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
import dev.kampr.terminal.view.CARET_SETTLE_MS
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val DEEP_ROWS = 200

// One test method per viewport, and not a loop over both. On wasm `runComposeUiTest` is
// asynchronous — it hands back a promise and the test framework only awaits the one the method
// *returns* — so a second call inside a loop runs detached and every assertion in it is thrown
// away. Measured: with the caret band deliberately re-broken, a looping version of this test
// passed.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.sweepsNothing(size: Pair<Dp, Dp>) {
    val pane = shellPane(rows = DEEP_ROWS, caretRow = 120)
    val session = PaneSession(BROWSER_PANE)
    browserTerminal(pane, session, size)
    assertTrue(
        session.view.maxScroll > DEEP_ROWS * session.grid.cellHeight * 0.5f,
        "the grid has to be far taller than the rectangle, or the excursion fits the band and " +
            "nothing is tested",
    )
    val resting = session.view.scrollY
    val steps = listOf(0, 40, 90, 150, 199, 120)
    repeat(3) {
        // Read at every step rather than at the end: a viewport dragged up the grid and back down
        // again finishes where it started, and the round trip is the flash the operator reported.
        assertEquals(
            steps.map { resting },
            steps.map { row -> caretTo(pane, row); session.view.scrollY },
            "the caret's round trip across the grid took the surface with it",
        )
    }
}

// #428 fixed the caret band against two JVM harnesses and said so in its own "not measured" line:
// the report came from **wasm**, and both proofs were JVM Compose. This is the same two defects
// asked of a real `TerminalView` inside a real ChromeHeadless, which is the machine the operator
// was on: "the terminal pane on wasm desktop keeps bouncing around and landing back where i last
// typed instead of the bottom of the screen".
@OptIn(ExperimentalTestApi::class)
class BrowserFollowingTest {
    @Test
    fun aRepaintThatSweepsTheCaretAcrossATallGridMovesNothingOnABrowserDesk() =
        runComposeUiTest { sweepsNothing(DESK) }

    // "mobile is similarly zoomed in a lot" — the same grid in a phone browser's rectangle.
    @Test
    fun aRepaintThatSweepsTheCaretAcrossATallGridMovesNothingOnABrowserPhone() =
        runComposeUiTest { sweepsNothing(PHONE) }

    // The other half of the same report, and the one that needs a hand: the wheel on a browser
    // desk. The band's floor used to be the drag's own clamp, so everything below the caret's
    // screenful was unreachable — and whatever a hand did win was thrown back by the next frame
    // that moved the caret.
    @Test
    fun aWheelReachesTheBottomOfTheGridInABrowserAndStaysThere() = runComposeUiTest {
        val pane = shellPane(rows = DEEP_ROWS, caretRow = 3)
        val session = PaneSession(BROWSER_PANE)
        browserTerminal(pane, session, DESK)
        assertTrue(
            session.view.band.floor > 0f,
            "the caret has to be holding the surface off the bottom of the grid, or nothing is tested",
        )

        repeat(200) {
            onRoot().performMouseInput {
                moveTo(Offset(width / 2f, height / 2f))
                scroll(1f, ScrollWheel.Vertical)
            }
            frames(1)
        }
        assertEquals(
            0f,
            session.view.scrollY,
            0.01f,
            "the wheel stopped ${session.view.scrollY}px above the bottom of the grid, at the " +
                "caret floor of ${session.view.band.floor}px",
        )
        assertTrue(!session.view.following, "the bottom of this grid is not the live edge")

        for (row in listOf(0, 90, 199, 3, 120)) {
            caretTo(pane, row)
            mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
            frames(4)
            assertEquals(
                0f,
                session.view.scrollY,
                0.01f,
                "the caret settling on row $row took the reader off the bottom of the grid and " +
                    "back to ${session.view.scrollY}px",
            )
        }
    }
}
