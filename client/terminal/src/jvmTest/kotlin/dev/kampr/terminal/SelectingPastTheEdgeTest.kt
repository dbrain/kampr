package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import kotlin.test.Test
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

private const val GRID = "Terminal grid"

private enum class Hand { MOUSE, FINGER }

// Holds the pointer above the top of the content for a while, the way a hand selecting a long
// answer waits for the pane to bring the rest of it in.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.holdAboveTheTop(hand: Hand) {
    val node = onNodeWithContentDescription(GRID, substring = true)
    when (hand) {
        Hand.MOUSE -> node.performMouseInput {
            moveTo(center)
            press()
            moveTo(center - Offset(0f, 100f))
            moveTo(Offset(center.x, 4f))
        }

        Hand.FINGER -> node.performTouchInput {
            down(center)
            advanceEventTime(1_000)
            moveTo(center - Offset(0f, 100f))
            moveTo(Offset(center.x, 4f))
        }
    }
    mainClock.advanceTimeBy(600)
    waitForIdle()
    when (hand) {
        Hand.MOUSE -> node.performMouseInput { release() }
        Hand.FINGER -> node.performTouchInput { up() }
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.selectsPastTheTop(hand: Hand) {
    val session = PaneSession(Phone.PANE)
    phoneTerminal(Phone.filled(rows = 200, caretRow = 199), session)
    val scrolled = session.view.scrollY
    val topRow = ((Phone.HEADER.value * density.density - session.grid.originY) / session.grid.cellHeight).toInt()

    holdAboveTheTop(hand)

    val selection = assertNotNull(session.view.selection, "$hand: the drag selected nothing")
    assertTrue(session.view.scrollY > scrolled, "$hand: holding at the top did not scroll the pane")
    assertTrue(
        selection.start.row < topRow,
        "$hand: the selection stopped at the edge of the screen: $selection, top row $topRow",
    )
}

// The operator: *"I want to be able to select text across a scroll"* — a selection dragged to the
// edge of the pane stopped at the edge, so text off the screen could only be had in pieces.
@OptIn(ExperimentalTestApi::class)
class SelectingPastTheEdgeTest {
    @Test
    fun aMouseSelectionHeldAtTheTopScrollsTheRestOfTheTextIn() = runComposeUiTest { selectsPastTheTop(Hand.MOUSE) }

    @Test
    fun aFingerSelectionHeldAtTheTopScrollsTheRestOfTheTextIn() = runComposeUiTest { selectsPastTheTop(Hand.FINGER) }
}
