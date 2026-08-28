package dev.kampr.terminal

import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.terminal.view.THUMB_TAG
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// The sheet's only zoom controls were three preset buttons — a jump each, with nothing in between.
// The slider is the continuous one, and it is hand-built because there is no Material on this
// classpath: `libs.versions.toml` carries runtime, foundation, ui and resources, and nothing else.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.openSheet(): PaneSession {
    val session = PaneSession(Phone.PANE)
    phoneTerminal(Phone.shell(rows = 40, caretRow = 6), session)
    session.view.sheetOpen = true
    waitForIdle()
    assertTrue(session.view.zoom > 0f, "the pane has to have taken a zoom, or nothing is tested")
    return session
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.dragSlider(by: Float) {
    onNodeWithContentDescription("Zoom level", substring = true).performTouchInput {
        down(center)
        moveTo(center + androidx.compose.ui.geometry.Offset(by, 0f))
        advanceEventTime(16)
        up()
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
class ZoomSliderTest {
    @Test
    fun draggingRightZoomsIn() = runComposeUiTest {
        val session = openSheet()
        val before = session.view.zoom
        dragSlider(60f)
        assertTrue(session.view.zoom > before, "the slider left the zoom at $before")
    }

    @Test
    fun draggingLeftZoomsOut() = runComposeUiTest {
        val session = openSheet()
        val before = session.view.zoom
        dragSlider(-60f)
        assertTrue(session.view.zoom < before, "the slider left the zoom at $before")
    }

    // The track is bounded by the same clamps `setZoom` applies, so running off the end lands on
    // the end rather than somewhere the grid cannot be drawn at.
    @Test
    fun runningOffTheEndStopsAtTheEnd() = runComposeUiTest {
        val session = openSheet()
        dragSlider(5000f)
        val ceiling = session.view.zoom
        dragSlider(5000f)
        assertEquals(ceiling, session.view.zoom, 0.001f, "the slider ran past its own maximum")
        assertTrue(ceiling > 0f, "the ceiling has to be a real zoom")
    }

    // Same contract a preset has: once the operator has picked, the default stops being re-derived
    // underneath them.
    @Test
    fun draggingIsTheOperatorChoosing() = runComposeUiTest {
        val session = openSheet()
        dragSlider(40f)
        assertTrue(session.view.chosen, "a slider drag has to stop the default being re-derived")
    }

    // The bug a browser found and every test here missed: the thumb was placed by a `layout` block
    // sitting after `.size(THUMB)`, so it was handed the thumb's own constraints and its travel was
    // always zero. The zoom moved, the semantics were right, and the thumb never left the left end.
    @Test
    fun theThumbMovesAlongTheTrackAsTheZoomChanges() = runComposeUiTest {
        val session = openSheet()
        val start = onNodeWithTag(THUMB_TAG, useUnmergedTree = true).fetchSemanticsNode().positionInRoot.x
        dragSlider(120f)
        val moved = onNodeWithTag(THUMB_TAG, useUnmergedTree = true).fetchSemanticsNode().positionInRoot.x
        assertTrue(
            moved > start + 4f,
            "the thumb sat at $start and is now at $moved, so it is not following the zoom",
        )
        dragSlider(-240f)
        val back = onNodeWithTag(THUMB_TAG, useUnmergedTree = true).fetchSemanticsNode().positionInRoot.x
        assertTrue(back < moved - 4f, "the thumb did not come back: $moved then $back")
    }

    // A pointer-driven control is invisible to a screen reader unless it says what it is worth,
    // and `Modifier.action` models a click only — there is no range semantics anywhere else in
    // this client, so this one carries its own.
    @Test
    fun theSliderSaysWhereItIs() = runComposeUiTest {
        val session = openSheet()
        val node = onNodeWithContentDescription("Zoom level", substring = true)
        node.assertExists()
        dragSlider(60f)
        node.assertExists()
    }
}
