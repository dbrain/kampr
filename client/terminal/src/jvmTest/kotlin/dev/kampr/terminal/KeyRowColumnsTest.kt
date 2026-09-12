package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
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
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.PaneKeyRow
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

private object ColumnsIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.keyRow(compact: Boolean) {
    val session = PaneSession(PANE)
    val tokens = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
        .let { KamprTokens(SoftTheme, it, typography(it, SoftTheme.label, TypeScale.Phone)) }
    setContent {
        CompositionLocalProvider(LocalTokens provides tokens) {
            Box(Modifier.fillMaxSize()) {
                PaneKeyRow(session, InputSink(PANE, ColumnsIo, session.latches), compact, enabled = true)
            }
        }
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.cap(spoken: String): Rect =
    onNodeWithContentDescription(spoken, substring = true).fetchSemanticsNode().boundsInRoot

// **A cap that spans two columns takes the gap between them with it**, so its row holds one fewer
// gap than the row above and every weight in it would grow by an eighth of one — which is the
// navigation group walking out of line with the row it is supposed to sit under. The blank slot
// beside shift used to hold that line; the separator holds it now, by giving back exactly what the
// merge saved. This is the measurement that says it lands, because nothing about the layout data
// can: the arithmetic is in `PaneKeyRow` and the error it makes is a couple of dp.
@OptIn(ExperimentalTestApi::class)
class KeyRowColumnsTest {
    @Test
    fun theArrowsStayInLineUnderAShiftThatSpansTwoColumns() {
        for (compact in listOf(false, true)) {
            runComposeUiTest {
                keyRow(compact)
                val up = cap("Up arrow")
                val down = cap("Down arrow")
                assertTrue(
                    abs(up.left - down.left) < 1f && abs(up.right - down.right) < 1f,
                    "compact=$compact: up spans ${up.left}..${up.right} and down ${down.left}..${down.right}",
                )
            }
        }
    }

    // And the cap itself is the two columns wide it is asking to be, gap included — a shift drawn
    // over one column with a hole beside it is the report, and a shift drawn over two minus the
    // gap is a cap that does not reach its neighbour.
    @Test
    fun shiftIsAsWideAsTheTwoColumnsItStandsIn() {
        for ((compact, pair) in listOf(false to ("Function" to "Keyboard"), true to ("Escape" to "Control"))) {
            runComposeUiTest {
                keyRow(compact)
                val shift = cap("Shift")
                val two = cap(pair.second).right - cap(pair.first).left
                // A pixel or two of slack, and no more: a weighted row hands out whole pixels, so
                // one box of weight two and two boxes of weight one need not come to the same
                // number — and the half gap each cap insets itself by rounds at both of its edges.
                // The defect this is guarding against is a whole gap, which is twice the slack.
                assertTrue(
                    abs(shift.width - two) < 3f,
                    "compact=$compact: shift is ${shift.width} wide where two columns are $two",
                )
            }
        }
    }
}
