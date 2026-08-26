package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

@OptIn(ExperimentalTestApi::class)
class HandScrollTest {
    // The report, verbatim: "scrolling on terminal screen opens keyboard, needs some kind of
    // swipe detection and only open keyboard on a tap". Two mechanisms, both here: a gesture whose
    // travel arrived with the release, and a touch that was catching a fling.
    // A flick fast enough to arrive as one move and a release used to read as a tap, because the
    // release was taken as the end of the gesture before the distance it carried was counted.
    @Test
    fun aFlickWhoseTravelArrivesWithTheReleaseIsNotATap() = runComposeUiTest {
        val pane = Phone.shell()
        val session = PaneSession(Phone.PANE)
        phoneTerminal(pane, session)

        onRoot().performTouchInput {
            down(Offset(centerX, centerY))
            advanceEventTime(8)
            moveBy(Offset(0f, 180f))
            up()
        }
        waitForIdle()

        assertFalse(session.keyboardOpen, "a flick raised the keyboard")
    }

    // Every scrolling surface treats a touch during a fling as a brake. This one took it as a tap
    // on the grid, so stopping a scroll you overshot cost you the keyboard.
    @Test
    fun aTapThatCatchesAFlingIsABrakeAndNotARequestForTheKeyboard() = runComposeUiTest {
        val pane = Phone.shell()
        val session = PaneSession(Phone.PANE)
        phoneTerminal(pane, session)

        session.view.velocityY = 900f
        onRoot().performTouchInput {
            down(Offset(centerX, centerY))
            advanceEventTime(16)
            up()
        }
        waitForIdle()

        assertFalse(session.keyboardOpen, "catching a fling raised the keyboard")
    }

    // A browser blurs the offscreen input on the pointer down that starts the scroll, and a desk
    // browser holds that focus without ever asking for a keyboard — so a flick that correctly
    // raises no keyboard still has to say it is over, or the pane is deaf until something taps it.
    @Test
    fun aFlickThatRaisesNoKeyboardStillSaysTheGestureIsOver() = runComposeUiTest {
        val pane = Phone.shell()
        val session = PaneSession(Phone.PANE)
        phoneTerminal(pane, session)
        val before = session.surfaceSettled

        onRoot().performTouchInput {
            down(Offset(centerX, centerY))
            advanceEventTime(8)
            moveBy(Offset(0f, 180f))
            up()
        }
        waitForIdle()

        assertFalse(session.keyboardOpen, "a flick raised the keyboard")
        assertTrue(session.surfaceSettled > before, "the flick left the offscreen input blurred")
    }
}
