package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.swipe
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneChrome
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.ui.keyboardInset
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.view.TerminalView
import kotlin.test.Test
import kotlin.test.assertTrue

private const val PANE = "01JKAMPRNODE0000000000000/w1:p1"

// A phone in portrait, and a header whose height the pane screen measures and hands down. Naming
// it here is what lets the assertion say "below the header" rather than "on screen somewhere".
private val HEADER = 96.dp
private val BARS = SafeArea(top = 32.dp, bottom = 46.dp)

// Gboard on a 1080x2400 phone. `bottom` is zero because the keyboard is drawn over the navigation
// bar — `KeyboardFloor` is what takes it, and SafeAreaValueTest pins that.
private val KEYBOARD = SafeArea(top = 32.dp, bottom = 0.dp, ime = 320.dp)

private object HushIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
}

private fun testTokens() = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    .let { KamprTokens(SoftTheme, it, typography(it, SoftTheme.label, TypeScale.Phone)) }

// A shell pane as herdr actually serves one: the desktop's row count, a few lines of output at the
// top, the caret on the last of them, and the whole rest of the grid blank. The caret is nowhere
// near the bottom of the grid, which is the case the bottom-pinned surface gets wrong.
private fun shellPane(rows: Int = 40, caretRow: Int = 3): PaneState {
    val pane = PaneState(PANE, StyleTable())
    val lines = (0..caretRow).map { "[20:36:31 dbrain@comingclean kampr]$ line $it" }
    pane.applyReset(
        ServerMsg.GridReset(
            pane = PANE,
            cols = 94,
            rows = rows,
            rowsData = lines.mapIndexed { index, text -> RowDiff(index, listOf(Run(0, text))) },
            cursor = Cursor(lines.last().length, caretRow, true),
            links = emptyList(),
        ),
    )
    return pane
}

// Composed the way the phone composes it: bars first, and whatever the caller does to them
// afterwards. The keyboard going up is a change to a surface that has already settled, which is
// the whole sequence — a pane composed with the keyboard already up never had a bottom to lose.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.terminal(
    pane: PaneState,
    session: PaneSession,
): MutableState<SafeArea> {
    val bars = mutableStateOf(BARS)
    setContent {
        CompositionLocalProvider(
            LocalTokens provides testTokens(),
            LocalPaneIo provides HushIo,
            LocalSafeArea provides bars.value,
            LocalPaneChrome provides PaneChrome(HEADER),
        ) {
            // The shape the phone stacks: the app root pays the keyboard once, and the pane fills
            // what is left. Nothing inside knows the keyboard is there — which is the whole point,
            // and the reason the surface has to notice that it got shorter.
            Box(Modifier.size(411.dp, 914.dp).keyboardInset()) {
                Box(Modifier.fillMaxSize()) { TerminalView(pane, session, HushIo) }
            }
        }
    }
    waitForIdle()
    return bars
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.caretTop(pane: PaneState, session: PaneSession): Dp {
    val probe = session.grid
    val index = pane.scrollback.historyRows + pane.cursor.row
    return with(density) { (probe.originY + index * probe.cellHeight).toDp() }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.stripTop(): Dp =
    onNodeWithContentDescription("Review this pane row by row").getUnclippedBoundsInRoot().top

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
                val pane = shellPane(caretRow = caretRow)
                val session = PaneSession(PANE)
                val bars = terminal(pane, session)
                bars.value = KEYBOARD
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
                    top >= HEADER,
                    "caretRow=$caretRow: the row being typed into is at $top, above the $HEADER " +
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
            val pane = shellPane(rows = 90, caretRow = 6)
            val session = PaneSession(PANE)
            terminal(pane, session)
            assertTrue(session.view.maxScroll > 0f, "the grid has to overflow, or nothing is tested")
            val top = caretTop(pane, session)
            assertTrue(
                top >= HEADER,
                "the caret is at $top, above the $HEADER header, with ${session.view.maxScroll}px " +
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
            val pane = shellPane(rows = 40, caretRow = 0)
            val session = PaneSession(PANE)
            terminal(pane, session)
            assertTrue(session.view.maxScroll > 0f, "the grid has to overflow, or nothing is tested")
            assertTrue(
                session.view.scrollY > 1f,
                "the caret at the top has to pull the surface up to it first",
            )
            pane.applyPatch(
                ServerMsg.GridPatch(
                    pane = PANE,
                    rows = listOf(RowDiff(36, listOf(Run(0, "> what would you like me to do?")))),
                    cursor = Cursor(2, 36, true),
                    links = emptyList(),
                ),
            )
            waitForIdle()
            val top = caretTop(pane, session)
            assertTrue(
                top >= HEADER && top <= stripTop(),
                "the caret moved to the bottom of the grid and the surface stayed on the banner: " +
                    "the row it is on is at $top, of a strip starting at ${stripTop()}",
            )
        }
    }

    // A reader who has dragged owns the viewport — until they type. Typing is a request to be
    // shown what you typed, and a surface that answers it only on the first frame answers it once.
    @Test
    fun typingAfterAReadHasScrolledBringsTheCaretBack() {
        runComposeUiTest {
            val pane = shellPane(rows = 90, caretRow = 6)
            val session = PaneSession(PANE)
            terminal(pane, session)
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
                    pane = PANE,
                    rows = listOf(RowDiff(7, listOf(Run(0, "$ echo typed")))),
                    cursor = Cursor(12, 7, true),
                    links = emptyList(),
                ),
            )
            waitForIdle()
            val top = caretTop(pane, session)
            assertTrue(
                top >= HEADER,
                "the caret came back to $top, above the $HEADER header, after the pane wrote to it",
            )
        }
    }
}
