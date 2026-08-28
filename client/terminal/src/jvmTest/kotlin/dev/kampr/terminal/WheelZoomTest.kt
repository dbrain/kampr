package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performMultiModalInput
import androidx.compose.ui.test.runComposeUiTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// A short grid, so the surface has somewhere to go on both axes and a zoom is visible in the pan.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.pane(): PaneSession {
    val session = PaneSession(Phone.PANE)
    phoneTerminal(Phone.shell(rows = 40, caretRow = 6), session)
    assertTrue(session.view.zoom > 0f, "the pane has to have taken a zoom, or nothing is tested")
    return session
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.wheel(notches: Float, held: Key? = null) {
    if (held == null) {
        onRoot().performMouseInput {
            moveTo(Offset(width / 2f, height / 2f))
            scroll(notches)
        }
    } else {
        onRoot().performMultiModalInput {
            key { keyDown(held) }
            mouse {
                moveTo(Offset(width / 2f, height / 2f))
                scroll(notches)
            }
            key { keyUp(held) }
        }
    }
    waitForIdle()
}

// The pinch branch in `terminalGestures` is gated on two *pressed* pointers, and a touchpad pinch
// arrives as a scroll with nothing pressed — so on a touchpad it was unreachable, and a mouse had
// no path at all. A browser delivers a touchpad pinch as a wheel with ctrl held, so the one branch
// serves the pinch and the mouse together. Same argument `WheelScrollTest` made for scrolling.
@OptIn(ExperimentalTestApi::class)
class WheelZoomTest {
    // Wheel away from the reader is `scroll(-1f)` — the direction `WheelScrollTest` walks into
    // history — and that is the direction every browser and every editor zooms *in*.
    @Test
    fun ctrlAndTheWheelZoomsIn() = runComposeUiTest {
        val session = pane()
        val before = session.view.zoom
        wheel(-1f, Key.CtrlLeft)
        assertTrue(
            session.view.zoom > before,
            "ctrl+wheel left the zoom at $before, so a touchpad and a mouse cannot zoom at all",
        )
    }

    @Test
    fun ctrlAndTheWheelTheOtherWayZoomsOut() = runComposeUiTest {
        val session = pane()
        val before = session.view.zoom
        wheel(1f, Key.CtrlLeft)
        assertTrue(session.view.zoom < before, "ctrl+wheel toward the reader left the zoom at $before")
    }

    // A mac sends the command key where everything else sends control, and a browser's synthetic
    // pinch uses ctrl on both.
    @Test
    fun theCommandKeyZoomsWhereControlDoes() = runComposeUiTest {
        val session = pane()
        val before = session.view.zoom
        wheel(-1f, Key.MetaLeft)
        assertTrue(session.view.zoom > before, "cmd+wheel left the zoom at $before")
    }

    // The whole reason it is a modifier and not the plain wheel: `WheelScrollTest` owns the plain
    // one, and a zoom that stole it would break every scroll that test pins.
    @Test
    fun aPlainWheelStillScrollsAndDoesNotZoom() = runComposeUiTest {
        val session = pane()
        val before = session.view.zoom
        wheel(-1f)
        assertEquals(before, session.view.zoom, 0.0001f, "the plain wheel zoomed instead of scrolling")
    }

    // Zoom goes through `setZoom`, so it is the operator's choice and the default stops being
    // re-derived under them — the same contract picking a preset has.
    @Test
    fun zoomingWithTheWheelIsTheOperatorChoosing() = runComposeUiTest {
        val session = pane()
        wheel(-1f, Key.CtrlLeft)
        assertTrue(session.view.chosen, "a wheel zoom has to stop the default being re-derived")
    }

    // One event is worth at most one step, exactly as `notches` clamps scrolling: a browser sends
    // around a hundred per notch and a precise trackpad sends a fraction of one.
    //
    // The `oneStep > 1f` assertion is the load-bearing one. Without it this test passes on a build
    // that cannot zoom at all — both ratios are 1.0 and the comparison holds — which is the shape
    // of harness this project deletes rather than keeps green.
    @Test
    fun oneEventIsWorthAtMostOneStep() = runComposeUiTest {
        val session = pane()
        val before = session.view.zoom
        wheel(-1f, Key.CtrlLeft)
        val oneStep = session.view.zoom / before
        assertTrue(oneStep > 1f, "a click has to move the zoom, or the clamp below tests nothing")

        val other = pane()
        val floor = other.view.zoom
        wheel(-500f, Key.CtrlLeft)
        assertEquals(
            oneStep,
            other.view.zoom / floor,
            0.001f,
            "a browser's hundred-per-notch delta threw the zoom across its whole range",
        )
    }

    // A fraction of a click is a fraction of a step, the way a precise trackpad sends it.
    @Test
    fun aFractionOfAClickIsAFractionOfAStep() = runComposeUiTest {
        val session = pane()
        val before = session.view.zoom
        wheel(-1f, Key.CtrlLeft)
        val full = session.view.zoom / before

        val other = pane()
        val floor = other.view.zoom
        wheel(-0.5f, Key.CtrlLeft)
        val half = other.view.zoom / floor
        assertTrue(half > 1f && half < full, "half a click moved $half where a whole one moved $full")
    }
}
