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

// **A karma test has two seconds and a real `TerminalView` spends them.** Each notch here is a
// synthesised mouse event plus a frame of a full pane, and a two-hundred-notch walk of the surface
// runs past mocha's per-test timeout on a loaded machine — the failure arrives as "Timeout of
// 2000ms exceeded", which reads like a hung test and is nothing but a long one. So a browser walk
// is sized to the distance it actually has to cover: forty notches is a hundred and twenty rows,
// which is the whole of either surface here twice over.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.wheel(notches: Int, towards: Float) {
    repeat(notches) {
        onRoot().performMouseInput {
            moveTo(Offset(width / 2f, height / 2f))
            scroll(towards, ScrollWheel.Vertical)
        }
        frames(1)
    }
}

private const val TO_THE_BOTTOM = 1f
private const val INTO_HISTORY = -1f
private const val TO_THE_END = 40

// One viewport per call and one call per `@Test`, for #433's reason.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.stopsAtTheRecord(size: Pair<Dp, Dp>) {
    val caret = 120
    val pane = shellPane(rows = DEEP_ROWS, caretRow = caret)
    val session = PaneSession(BROWSER_PANE)
    browserTerminal(pane, session, size)
    val view = session.view
    assertEquals(
        (DEEP_ROWS - 1 - caret) * session.grid.cellHeight,
        view.contentFloor,
        0.01f,
        "the floor has to be the last written row on the bottom of the rectangle",
    )
    assertTrue(
        view.contentFloor > 0f && view.contentFloor < view.maxScroll,
        "there has to be blank tail below the record (${view.contentFloor}) and travel above it " +
            "(${view.maxScroll}), or nothing is tested",
    )

    // Up into the record first, so that coming back down is a distance actually travelled: a floor
    // that never let go of the surface at all would pass an assertion made from where it opened.
    wheel(10, INTO_HISTORY)
    assertTrue(
        view.scrollY > view.contentFloor,
        "the wheel could not walk up into the record, so coming back down proves nothing",
    )

    wheel(TO_THE_END, TO_THE_BOTTOM)
    assertEquals(
        view.contentFloor,
        view.scrollY,
        0.01f,
        "the wheel came to rest ${view.contentFloor - view.scrollY}px from the end of the record, " +
            "in the blank tail below it",
    )
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
    //
    // Written to the last row of the grid, so that the end of the record and the end of the grid
    // are the same place and this stays a test about the caret's floor alone.
    @Test
    fun aWheelReachesTheEndOfTheRecordInABrowserAndStaysThere() = runComposeUiTest {
        val pane = writtenPane(rows = DEEP_ROWS, caretRow = 120)
        val session = PaneSession(BROWSER_PANE)
        browserTerminal(pane, session, DESK)
        assertTrue(
            session.view.band.floor > 0f,
            "the caret has to be holding the surface off the bottom of the grid, or nothing is tested",
        )
        assertEquals(0f, session.view.contentFloor, 0.01f, "this pane is written to its last row")

        wheel(TO_THE_END, TO_THE_BOTTOM)
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

    // And the regression that reachable bottom brought with it, on the platform the whole report
    // came from: a herdr pane is as tall as the desk made it, so on a pane the shell has not filled
    // the bottom of the grid is eighty rows of nothing below the end of the record. A hand may go
    // to the end of what there is to read and no further.
    @Test
    fun aWheelStopsAtTheEndOfTheRecordOnABrowserDesk() = runComposeUiTest {
        stopsAtTheRecord(DESK)
    }

    // "mobile is similarly zoomed in a lot", again — the same pane in a phone browser's rectangle.
    @Test
    fun aWheelStopsAtTheEndOfTheRecordOnABrowserPhone() = runComposeUiTest {
        stopsAtTheRecord(PHONE)
    }
}
