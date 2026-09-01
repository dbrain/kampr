package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val GRID = "Terminal grid"

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.dragTheMouse(fromX: Float, toX: Float) {
    onNodeWithContentDescription(GRID, substring = true).performMouseInput {
        val y = center.y
        moveTo(Offset(fromX, y))
        press()
        moveTo(Offset((fromX + toX) / 2f, y))
        moveTo(Offset(toX, y))
        release()
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.clickTheMouse(x: Float) {
    onNodeWithContentDescription(GRID, substring = true).performMouseInput {
        moveTo(Offset(x, center.y))
        press()
        release()
    }
    waitForIdle()
}

// A mouse is not a fingertip. Selecting on this surface cost a long press — a press held still for
// half a second, which is a gesture a desk has no name for and no reason to guess at — and a
// mouse drag panned the grid instead, on a surface whose wheel already pans it.
@OptIn(ExperimentalTestApi::class)
class DeskSelectionTest {
    @Test
    fun aMouseDragAcrossTheGridSelectsWithoutBeingHeldStill() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        phoneTerminal(Phone.shell(), session)
        dragTheMouse(fromX = 60f, toX = 300f)

        val selection = assertNotNull(session.view.selection, "a mouse drag selected nothing at all")
        assertTrue(
            selection.end.col > selection.start.col,
            "the drag selected one cell and went nowhere: $selection",
        )
        onNodeWithContentDescription("Copy the selection").assertExists()
    }

    // The grid pans with the wheel (`terminalWheel`), which is how a terminal emulator does it, so
    // giving the drag to the selection takes nothing away from a desk.
    @Test
    fun aMouseDragDoesNotAlsoPanTheSurface() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        phoneTerminal(Phone.shell(), session)
        val panX = session.view.panX
        val scrollY = session.view.scrollY
        dragTheMouse(fromX = 300f, toX = 60f)
        assertEquals(panX, session.view.panX, "the drag panned the surface sideways as well")
        assertEquals(scrollY, session.view.scrollY, "the drag scrolled the surface as well")
    }

    // A click is not a drag: it still raises the keyboard, and it still puts a selection away —
    // the one gesture that clears the pill on every device.
    @Test
    fun aMouseClickThatNeverMovedIsStillATap() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        phoneTerminal(Phone.shell(), session)
        clickTheMouse(200f)
        assertNull(session.view.selection, "a click on its own started a selection")
        assertTrue(session.keyboardOpen, "a click on the grid no longer asks for the keyboard")

        dragTheMouse(fromX = 60f, toX = 300f)
        assertNotNull(session.view.selection, "the drag selected nothing")
        clickTheMouse(200f)
        assertNull(session.view.selection, "a click left the selection standing")
    }

    // The finger's rules are untouched: a drag is a pan, and only a press held still selects.
    @Test
    fun aFingerDragStillPansRatherThanSelecting() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        phoneTerminal(Phone.shell(), session)
        onNodeWithContentDescription(GRID, substring = true).performTouchInput {
            val y = center.y
            down(Offset(300f, y))
            moveTo(Offset(200f, y))
            moveTo(Offset(60f, y))
            up()
        }
        waitForIdle()
        assertNull(session.view.selection, "a finger drag selected instead of panning")
    }
}
