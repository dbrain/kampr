package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.ScrollWheel
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performMultiModalInput
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.terminal.view.WHEEL_ROWS
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// A grid taller than the window, which is the ordinary case for a phone-sized viewport against a
// desktop-sized pane and the only one where scrolling means anything.
//
// Written to the last row of it, because a wheel goes as far as there is anything to read and no
// further: the same 90-row grid with seven lines at the top of it has its whole record on the
// screen at once and a wheel that moves it anywhere would be moving it through blank tail.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.deepPane(): PaneSession {
    val session = PaneSession(Phone.PANE)
    phoneTerminal(Phone.filled(rows = 90, caretRow = 6), session)
    assertTrue(session.view.maxScroll > 0f, "the grid has to overflow, or nothing is tested")
    return session
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.wheel(notches: Float, axis: ScrollWheel = ScrollWheel.Vertical) {
    onRoot().performMouseInput {
        moveTo(Offset(width / 2f, height / 2f))
        scroll(notches, axis)
    }
    waitForIdle()
}

// A wheel over the grid did nothing at all: the surface has a touch drag, a pinch and a long
// press, and no pointer-scroll path — so a desktop browser, which is the only place the web
// client runs, could not move the pane without a touchscreen.
private const val NOTCHES_TO_THE_END = 40

@OptIn(ExperimentalTestApi::class)
class WheelScrollTest {
    @Test
    fun aWheelBackwardsWalksIntoHistory() = runComposeUiTest {
        val session = deepPane()
        val floor = session.view.scrollY
        wheel(-1f)
        assertTrue(
            session.view.scrollY > floor,
            "the wheel left the surface at $floor, where the drag would have moved it",
        )
        assertEquals(
            floor + WHEEL_ROWS * session.grid.cellHeight,
            session.view.scrollY,
            0.01f,
            "a notch is meant to be $WHEEL_ROWS rows of this grid",
        )
        assertTrue(!session.view.following, "the reader took the viewport, so it stops following")
    }

    // A terminal scrolls by the row and nothing smaller. A precise trackpad sends fractions of a
    // click and a browser sends the raw DOM delta, and moving the surface by each of those as it
    // came glided it through positions where every row on screen was cut through its glyphs —
    // the operator's *"scrolling on desktop is real smooth scrolling, which feels wrong"*. The
    // fractions are kept and spent a whole row at a time, and one event is still worth at most a
    // notch, because a hundred-per-notch number is the host's unit and not one this surface reads.
    @Test
    fun aWheelMovesWholeRowsAndNeverSlicesOne() = runComposeUiTest {
        val session = deepPane()
        val cell = session.grid.cellHeight
        val floor = session.view.scrollY
        repeat(4) {
            wheel(-0.25f)
            val rows = (session.view.scrollY - floor) / cell
            assertEquals(
                kotlin.math.round(rows),
                rows,
                0.001f,
                "a quarter click left the surface ${rows} rows off the floor, part way through a row",
            )
        }
        assertEquals(
            floor + WHEEL_ROWS * cell,
            session.view.scrollY,
            0.01f,
            "four quarter clicks are one notch, and a notch is $WHEEL_ROWS rows",
        )
        val notch = session.view.scrollY
        wheel(-500f)
        assertEquals(
            notch + WHEEL_ROWS * cell,
            session.view.scrollY,
            0.01f,
            "a browser's hundred-per-notch delta threw the pane across its history",
        )
    }

    // Everything `scrollBy` owns, owned once: the reader has taken the viewport, so the opening
    // scroll stops being re-derived under them exactly as it does after a drag.
    @Test
    fun aWheelClaimsTheViewportTheWayADragDoes() = runComposeUiTest {
        val session = deepPane()
        wheel(-1f)
        assertTrue(session.view.scrolled, "a wheel is the reader moving the surface, same as a drag")
    }

    // Both ends of the surface, and the far end is the one that was missing. The wheel used to be
    // clamped at the caret floor — a *resting* place, never a limit — so on any grid taller than
    // the viewport whose caret is above the bottom of it, the last rows of the pane could not be
    // reached at all. `ReachingTheBottomOfAPaneTest` is what that costs an operator; this is the
    // wheel's share of it.
    @Test
    fun aWheelReachesBothEndsOfTheSurface() = runComposeUiTest {
        val session = deepPane()
        val view = session.view
        assertTrue(view.band.floor > 0f, "the caret floor has to be off the bottom, or nothing is tested")
        repeat(NOTCHES_TO_THE_END) { wheel(-1f) }
        assertEquals(view.maxScroll, view.scrollY, 0.01f, "the wheel ran past the top of history")
        repeat(NOTCHES_TO_THE_END) { wheel(1f) }
        assertEquals(0f, view.scrollY, 0.01f, "the wheel stopped short of the bottom of the grid")
    }

    // The grid pans sideways as well, and a trackpad and a browser both send that axis of their
    // own accord. A short grid is the case with room to pan: the opening zoom fits the taller of
    // the two axes, so a 90-row pane is fitted to the width and has no sideways room at all.
    @Test
    fun aSidewaysWheelPansTheGrid() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        phoneTerminal(Phone.shell(rows = 8, caretRow = 3), session)
        val view = session.view
        assertTrue(view.minPanX < -50f, "the grid has to overrun the window sideways, or nothing is tested")
        val before = view.panX
        wheel(1f, ScrollWheel.Horizontal)
        assertTrue(view.panX < before, "the sideways wheel left the pan at $before")
        assertEquals(
            (before - WHEEL_ROWS * session.grid.cellWidth).coerceIn(view.minPanX, 0f),
            view.panX,
            0.01f,
            "a sideways notch is meant to be $WHEEL_ROWS columns",
        )
        repeat(NOTCHES_TO_THE_END) { wheel(-1f, ScrollWheel.Horizontal) }
        assertEquals(0f, view.panX, 0.01f, "the pan ran past the left edge of the grid")
    }

    // A desktop toolkit leaves shift+wheel on the vertical axis and expects the surface to read
    // the modifier; a browser has already turned it into deltaX. Both have to pan.
    @Test
    fun shiftPutsTheWheelOnTheOtherAxis() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        phoneTerminal(Phone.shell(rows = 8, caretRow = 3), session)
        val view = session.view
        val before = view.panX
        onRoot().performMultiModalInput {
            key { keyDown(Key.ShiftLeft) }
            mouse {
                moveTo(Offset(width / 2f, height / 2f))
                scroll(1f)
            }
            key { keyUp(Key.ShiftLeft) }
        }
        waitForIdle()
        assertEquals(
            (before - WHEEL_ROWS * session.grid.cellWidth).coerceIn(view.minPanX, 0f),
            view.panX,
            0.01f,
            "shift+wheel did not reach the horizontal axis",
        )
    }
}
