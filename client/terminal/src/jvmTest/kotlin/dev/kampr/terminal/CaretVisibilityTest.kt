package dev.kampr.terminal

import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.swipe
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.view.CARET_SETTLE_MS
import kotlin.test.Test
import kotlin.test.assertTrue

private val PHONE_WIDTH = 411.dp

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.caretTop(pane: PaneState, session: PaneSession): Dp =
    rowTop(pane, session, pane.cursor.row)

// The report, verbatim: "still can't see what im typing on the terminal (but i can see letters
// show up on the desktop looking at the terminal)". The keystrokes reach the PTY; the row they
// land on is off the top of the surface.
//
// The surface pins the *bottom of the grid* to the bottom of the content rectangle. A herdr pane
// is as tall as the desktop made it and the caret sits wherever the shell left it — so the moment
// the rectangle is shorter than the grid, everything above the fold goes, caret first. The
// keyboard is what makes the rectangle shorter, every time, on every device.
@OptIn(ExperimentalTestApi::class)
class CaretVisibilityTest {
    @Test
    fun theCaretStaysOnScreenWhenTheKeyboardTakesHalfTheWindow() {
        for (caretRow in listOf(0, 3, 12)) {
            runComposeUiTest {
                val pane = Phone.shell(caretRow = caretRow)
                val session = PaneSession(Phone.PANE)
                val bars = phoneTerminal(pane, session)
                bars.value = Phone.KEYBOARD
                waitForIdle()
                val top = caretTop(pane, session)
                assertTrue(
                    session.grid.cellHeight > 1f,
                    "the grid never laid out, so nothing was measured",
                )
                assertTrue(
                    session.view.maxScroll > 0f,
                    "caretRow=$caretRow: the grid fits the window with the keyboard up, so the " +
                        "case this is about never arose",
                )
                assertTrue(
                    top >= Phone.HEADER,
                    "caretRow=$caretRow: the row being typed into is at $top, above the ${Phone.HEADER} " +
                        "header — every character typed lands where nothing can see it",
                )
                assertTrue(
                    top + with(density) { session.grid.cellHeight.toDp() } <= stripTop(),
                    "caretRow=$caretRow: the row being typed into is behind the column strip",
                )
            }
        }
    }

    // The same surface with no keyboard: a grid taller than the rectangle it is shown in is the
    // ordinary case for a phone against a desktop-sized pane, and the caret has to survive it.
    @Test
    fun theCaretStaysOnScreenWhenTheGridIsTallerThanTheViewport() {
        runComposeUiTest {
            val pane = Phone.shell(rows = 90, caretRow = 6)
            val session = PaneSession(Phone.PANE)
            phoneTerminal(pane, session)
            assertTrue(session.view.maxScroll > 0f, "the grid has to overflow, or nothing is tested")
            val top = caretTop(pane, session)
            assertTrue(
                top >= Phone.HEADER,
                "the caret is at $top, above the ${Phone.HEADER} header, with ${session.view.maxScroll}px " +
                    "of scroll left unused below it",
            )
        }
    }

    // Caught on a real device, against a real `claude` starting in a real pane: the banner is drawn
    // with the caret still at the top of the grid, and a frame later the caret is down in the input
    // box. A floor that only ever rises leaves the surface parked on the banner for the rest of the
    // session — the input box, and everything typed into it, off the bottom this time.
    //
    // So the floor is where a reader who has not scrolled *is*, not merely where they may not go
    // below. The live edge of a terminal moves both ways.
    @Test
    fun aCaretThatVisitsTheTopDoesNotStrandTheSurfaceThere() {
        runComposeUiTest {
            val pane = Phone.shell(rows = 40, caretRow = 0)
            val session = PaneSession(Phone.PANE)
            phoneTerminal(pane, session)
            assertTrue(session.view.maxScroll > 0f, "the grid has to overflow, or nothing is tested")
            assertTrue(
                session.view.scrollY > 1f,
                "the caret at the top has to pull the surface up to it first",
            )
            pane.applyPatch(
                ServerMsg.GridPatch(
                    pane = Phone.PANE,
                    rows = listOf(RowDiff(36, listOf(Run(0, "> what would you like me to do?")))),
                    cursor = Cursor(2, 36, true),
                    links = emptyList(),
                ),
            )
            // The band is moved by where the caret *stops*, so the caret has to stop. A frame is
            // not enough on purpose: a repaint that walks the caret across the grid and back
            // inside one batch of writes must move nothing at all (`FollowingTheOutputTest`).
            mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
            waitForIdle()
            val top = caretTop(pane, session)
            assertTrue(
                top >= Phone.HEADER && top <= stripTop(),
                "the caret moved to the bottom of the grid and the surface stayed on the banner: " +
                    "the row it is on is at $top, of a strip starting at ${stripTop()}",
            )
        }
    }

    // The same rule on the other axis, and it was gated on the wrong question. A pane at a zoom
    // the operator picked overflows sideways as well as downwards, and the sideways chase asked
    // whether the surface sat at scroll *zero* — which stopped being where a follower rests at
    // #175, and stopped being it for good with the caret band. So on every pane that overflows
    // both axes, which is the ordinary phone case with the keyboard up, the caret could sit two
    // screen widths off the right edge and typing never brought it back.
    @Test
    fun theCaretStaysOnScreenSidewaysWhenTheGridOverflowsBothAxes() {
        runComposeUiTest {
            val pane = Phone.shell(rows = 40, caretRow = 2)
            val session = PaneSession(Phone.PANE)
            val bars = phoneTerminal(pane, session, io = ReadableIo)
            bars.value = Phone.KEYBOARD
            waitForIdle()
            pane.applyPatch(
                ServerMsg.GridPatch(
                    pane = Phone.PANE,
                    rows = listOf(RowDiff(2, listOf(Run(0, "$ " + "x".repeat(88))))),
                    cursor = Cursor(90, 2, true),
                    links = emptyList(),
                ),
            )
            waitForIdle()
            assertTrue(session.view.minPanX < 0f, "the grid has to overrun the window sideways")
            assertTrue(session.view.band.floor > 0f, "and downwards, or the old gate would have held")
            val left = caretLeft(pane, session)
            assertTrue(
                left >= 0.dp && left <= PHONE_WIDTH,
                "the column being typed into is at $left, off a ${PHONE_WIDTH} window — every " +
                    "character typed lands where nothing can see it",
            )
        }
    }

    // A reader who has dragged owns the viewport — until they type. Typing is a request to be
    // shown what you typed, and a surface that answers it only on the first frame answers it once.
    @Test
    fun typingAfterAReadHasScrolledBringsTheCaretBack() {
        runComposeUiTest {
            val pane = Phone.shell(rows = 90, caretRow = 6)
            val session = PaneSession(Phone.PANE)
            phoneTerminal(pane, session)
            onRoot().performTouchInput {
                swipe(
                    start = Offset(centerX, centerY - 200f),
                    end = Offset(centerX, centerY + 200f),
                    durationMillis = 200,
                )
            }
            waitForIdle()
            pane.applyPatch(
                ServerMsg.GridPatch(
                    pane = Phone.PANE,
                    rows = listOf(RowDiff(7, listOf(Run(0, "$ echo typed")))),
                    cursor = Cursor(12, 7, true),
                    links = emptyList(),
                ),
            )
            waitForIdle()
            val top = caretTop(pane, session)
            assertTrue(
                top >= Phone.HEADER,
                "the caret came back to $top, above the ${Phone.HEADER} header, after the pane wrote to it",
            )
        }
    }
}
