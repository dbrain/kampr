package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.runComposeUiTest
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
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.PaneKeyRow
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertTrue

private val BARS = SafeArea(top = 32.dp, bottom = 46.dp)

// Rotated, with three-button navigation: the strip the system reserves leaves the bottom of the
// window and lands on one side of it, which is where the outermost caps are.
private val SIDE_BARS = listOf(
    SafeArea(top = 24.dp, bottom = 0.dp, left = 48.dp, right = 0.dp),
    SafeArea(top = 24.dp, bottom = 0.dp, left = 0.dp, right = 48.dp),
)

// Taller than the gesture handle on purpose: a bottom navigation bar is what actually sits under
// the key row on the pane screen, and it already keeps its own labels clear of the handle.
private val NAV = 70.dp

private const val PANE = "01JNODE/w1:p1"

private object SilentIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
}

private fun testTokens() = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    .let { KamprTokens(SoftTheme, it, typography(it, SoftTheme.label, TypeScale.Phone)) }

private val LAST_ROW = listOf("End", "Left arrow", "Down arrow", "Right arrow")

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.lowestCap(): Dp = LAST_ROW
    .flatMap { onAllNodesWithContentDescription(it, substring = true).fetchSemanticsNodes() }
    .also { assertTrue(it.isNotEmpty(), "no key caps on screen, so nothing was measured") }
    .maxOf { with(density) { it.boundsInRoot.bottom.toDp() } }

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.keyRow(compact: Boolean, below: Dp, bars: SafeArea = BARS) {
    val pane = PaneState(PANE, StyleTable())
    val session = PaneSession(PANE)
    setContent {
        CompositionLocalProvider(LocalTokens provides testTokens(), LocalSafeArea provides bars) {
            Column(Modifier.fillMaxSize()) {
                Box(Modifier.weight(1f))
                PaneKeyRow(session, InputSink(pane.id, SilentIo, session.latches), compact, enabled = true)
                if (below > 0.dp) Box(Modifier.height(below))
            }
        }
    }
    waitForIdle()
}

// The key row docks above whatever the system is drawing at the bottom of the window — and above
// nothing at all when something else already covers it. Both halves matter: padding for a gesture
// handle that a navigation bar has already cleared leaves a dead strip under the last row of keys.
@OptIn(ExperimentalTestApi::class)
class KeyRowSafeAreaTest {
    @Test
    fun theKeyRowClearsTheGestureHandleWhenItIsTheLastThingInTheWindow() {
        for (compact in listOf(false, true)) {
            runComposeUiTest {
                keyRow(compact, below = 0.dp)
                val screen = onRoot().getUnclippedBoundsInRoot()
                assertTrue(
                    lowestCap() <= screen.bottom - BARS.bottom,
                    "compact=$compact: the keys reach ${lowestCap()} of ${screen.bottom}, inside " +
                        "the ${BARS.bottom} the system draws its gesture handle in",
                )
            }
        }
    }

    @Test
    fun theKeyRowClearsTheBarsThatMoveToTheSideInLandscape() {
        for (bars in SIDE_BARS) {
            runComposeUiTest {
                keyRow(compact = true, below = 0.dp, bars = bars)
                val screen = onRoot().getUnclippedBoundsInRoot()
                val caps = LAST_ROW
                    .flatMap { onAllNodesWithContentDescription(it, substring = true).fetchSemanticsNodes() }
                    .also { assertTrue(it.isNotEmpty(), "no key caps on screen, so nothing was measured") }
                    .map { with(density) { it.boundsInRoot.left.toDp() to it.boundsInRoot.right.toDp() } }
                assertTrue(
                    caps.minOf { it.first } >= bars.left,
                    "$bars: a cap starts at ${caps.minOf { it.first }}, inside the ${bars.left} " +
                        "the system draws in on the left",
                )
                assertTrue(
                    caps.maxOf { it.second } <= screen.right - bars.right,
                    "$bars: a cap reaches ${caps.maxOf { it.second }} of ${screen.right}, inside " +
                        "the ${bars.right} the system draws in on the right",
                )
            }
        }
    }

    @Test
    fun theKeyRowAddsNothingWhenSomethingElseAlreadyCoversTheHandle() {
        for (compact in listOf(false, true)) {
            runComposeUiTest {
                keyRow(compact, below = NAV)
                val screen = onRoot().getUnclippedBoundsInRoot()
                val gap = screen.bottom - NAV - lowestCap()
                assertTrue(
                    abs(gap.value) < 1f,
                    "compact=$compact: $gap of dead strip between the last key and the $NAV of " +
                        "navigation bar that is already holding the handle off",
                )
            }
        }
    }
}
