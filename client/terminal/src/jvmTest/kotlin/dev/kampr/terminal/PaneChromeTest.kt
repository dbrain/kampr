package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipe
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.view.TerminalView
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val PANE = "01JKAMPRNODE0000000000000/w9:p1"

private object QuietIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
}

private fun filledPane(): PaneState {
    val pane = PaneState(PANE, StyleTable())
    val lines = (1..24).map { "row $it ${"-".repeat(40)}" }
    pane.applyReset(
        ServerMsg.GridReset(
            pane = PANE,
            cols = 62,
            rows = 24,
            rowsData = lines.mapIndexed { index, text -> RowDiff(index, listOf(Run(0, text))) },
            cursor = Cursor(lines.last().length, 23, true),
            links = emptyList(),
        ),
    )
    return pane
}

private fun tokens(): KamprTokens {
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    return KamprTokens(SoftTheme, fonts, typography(fonts, SoftTheme.label, TypeScale.Phone))
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), LocalPaneIo provides QuietIo) {
        Box(Modifier.fillMaxSize()) { content() }
    }
}

// The bottom row of a shell pane is the prompt, and every character typed into it lands there. It
// is the one row that must never be behind the chrome.
@OptIn(ExperimentalTestApi::class)
class PaneChromeTest {
    @Test
    fun thePaintStopsExactlyWhereTheStripStarts() = runComposeUiTest {
        val session = PaneSession(PANE)
        setContent {
            CompositionLocalProvider(LocalSafeArea provides SafeArea(top = 32.dp, bottom = 46.dp)) {
                Themed { TerminalView(filledPane(), session, QuietIo) }
            }
        }
        waitForIdle()
        val root = onRoot().getUnclippedBoundsInRoot()
        // The pill is the tallest thing in the strip; the Row pads 4.dp above it and 6.dp below.
        val pill = onNodeWithContentDescription("Review this pane row by row")
            .getUnclippedBoundsInRoot()
        val stripTop = pill.top - 4.dp
        // What the grid stops painting at: the chrome the strip stands off, plus the strip.
        val reserved = root.bottom - 46.dp - with(density) { session.indicatorHeight.toDp() }
        assertTrue(
            abs((stripTop - reserved).value) < 1f,
            "the grid stops painting at $reserved and the strip starts at $stripTop — short of it " +
                "and the bottom row, which is the prompt, is behind the strip; past it and the " +
                "chrome is counted twice and the rows are pushed off the top",
        )
    }
}

// The sheet's scrim carried a semantic action and no pointer handler, so a tap outside it was not
// a dismissal at all — it fell through to the grid underneath, which reads a tap as "raise the
// keyboard". The sheet stayed up and the pane went into typing mode at the same time.
@OptIn(ExperimentalTestApi::class)
class ZoomSheetDismissTest {
    @Test
    fun tappingOutsideTheSheetDismissesItAndNothingElse() = runComposeUiTest {
        val session = PaneSession(PANE)
        setContent { Themed { TerminalView(filledPane(), session, QuietIo) } }
        waitForIdle()
        session.view.sheetOpen = true
        waitForIdle()
        // Tapped near the top of the scrim rather than at its centre. The scrim fills the window,
        // so its centre moved *inside* the sheet once that grew a slider and a resize panel — and
        // a tap landing on the sheet is not the thing this test is about.
        onNodeWithContentDescription("Close the zoom sheet").performTouchInput {
            down(Offset(width / 2f, 8f))
            up()
        }
        waitForIdle()
        assertFalse(session.view.sheetOpen, "a tap on the scrim has to close the sheet")
        assertFalse(
            session.keyboardOpen,
            "and must not reach the grid underneath, which reads a tap as a request to type",
        )
    }

    // The scrim covers the sheet as well as the screen, so it has to sit under it rather than
    // beside it: picking a zoom must not also be a dismissal.
    @Test
    fun tappingAControlInsideTheSheetIsNotADismissal() = runComposeUiTest {
        val session = PaneSession(PANE)
        setContent { Themed { TerminalView(filledPane(), session, QuietIo) } }
        waitForIdle()
        session.view.sheetOpen = true
        waitForIdle()
        onNodeWithContentDescription("Close up", substring = true).performClick()
        waitForIdle()
        assertTrue(session.view.sheetOpen, "picking a zoom is not a request to close the sheet")
    }
}

// A long press is a press that stayed still. The detector only ever asked how long the finger had
// been down, so anything slower than the long-press timeout — every pinch, and every unhurried
// drag — was taken for a selection and the zoom never moved.
@OptIn(ExperimentalTestApi::class)
class TerminalGestureTest {
    @Test
    fun anUnhurriedDragIsNotASelection() = runComposeUiTest {
        val session = PaneSession(PANE)
        setContent { Themed { TerminalView(filledPane(), session, QuietIo) } }
        waitForIdle()
        onNodeWithContentDescription("Terminal grid", substring = true).performTouchInput {
            down(center)
            moveTo(center + Offset(0f, -40f))
            advanceEventTime(900)
            moveTo(center + Offset(0f, -200f))
            up()
        }
        waitForIdle()
        assertNull(
            session.view.selection,
            "a drag that outlasted the long-press timeout was taken for a selection, which is what " +
                "a two-finger pinch always is",
        )
    }

    @Test
    fun aPressThatStaysStillIsStillASelection() = runComposeUiTest {
        val session = PaneSession(PANE)
        setContent { Themed { TerminalView(filledPane(), session, QuietIo) } }
        waitForIdle()
        onNodeWithContentDescription("Terminal grid", substring = true).performTouchInput {
            down(center)
            advanceEventTime(900)
            moveTo(center)
            up()
        }
        waitForIdle()
        assertNotNull(session.view.selection, "holding still is how a selection starts")
    }
}

private class PrefsIo : PaneIo {
    var values by mutableStateOf<Map<String, String>>(emptyMap())
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs(values)
}

private fun historicPane(): PaneState {
    val pane = filledPane()
    pane.applyScrollback(
        ServerMsg.Scrollback(
            pane = PANE,
            fromTop = 0,
            rows = (0 until 400).map { RowDiff(it, listOf(Run(0, "history $it ${"=".repeat(40)}"))) },
            totalRows = 400,
            complete = false,
            capped = true,
        ),
    )
    return pane
}

// A frame arriving is not an instruction to move the reader. `prefs` is a key of the effect that
// places the opening scroll, and that effect used to be guarded only by whether a zoom had been
// picked — which a reader who has only ever dragged never does.
@OptIn(ExperimentalTestApi::class)
class ScrollAnchorTest {
    @Test
    fun aPrefsFrameArrivingMidReadDoesNotMoveTheViewport() = runComposeUiTest {
        val session = PaneSession(PANE)
        val io = PrefsIo()
        val pane = historicPane()
        setContent {
            CompositionLocalProvider(LocalSafeArea provides SafeArea(top = 32.dp, bottom = 46.dp)) {
                CompositionLocalProvider(LocalTokens provides tokens(), LocalPaneIo provides io) {
                    Box(Modifier.fillMaxSize()) { TerminalView(pane, session, io) }
                }
            }
        }
        waitForIdle()
        val opened = session.view.scrollY

        // Down, because down is into history: the surface follows the finger and dragging the
        // sheet down uncovers what was above it.
        onRoot().performTouchInput {
            swipe(
                start = Offset(centerX, centerY - 200f),
                end = Offset(centerX, centerY + 200f),
                durationMillis = 200,
            )
        }
        waitForIdle()

        val parked = session.view.scrollY
        assertTrue(
            abs(parked - opened) > 1f,
            "the swipe did not move the viewport ($opened -> $parked), maxScroll=${session.view.maxScroll}",
        )

        io.values = mapOf("zoom" to "1.0")
        waitForIdle()

        assertTrue(
            abs(session.view.scrollY - parked) < 1f,
            "a prefs frame moved the reader from $parked to ${session.view.scrollY}",
        )
    }
}
