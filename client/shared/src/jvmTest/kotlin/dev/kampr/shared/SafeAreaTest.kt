package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.BottomNav
import dev.kampr.shared.ui.BottomSheet
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.FallbackSurfaces
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.HerdLandscape
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.screenInset
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.ui.Tab
import dev.kampr.shared.ui.named
import kotlin.test.Test
import kotlin.test.assertTrue

// A gesture handle's worth of system furniture, and a status bar's. Real values on the API 37
// AVD this was found on: 1080×2400 at 420 dpi.
private val BARS = SafeArea(top = 32.dp, bottom = 46.dp)

private const val PANE_ID = "01JNODE/w1:p1"
private const val SHEET_FLOOR = "Sheet floor"

// Rotated, with three-button navigation: the status bar thins, the gesture area goes to whichever
// side the rotation put it on, and the bottom of the window is no longer where the system draws.
private val SIDE_BARS = listOf(
    SafeArea(top = 24.dp, bottom = 0.dp, left = 48.dp, right = 0.dp),
    SafeArea(top = 24.dp, bottom = 0.dp, left = 0.dp, right = 48.dp),
)

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Bars(bars: SafeArea = BARS, content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), LocalSafeArea provides bars, content = content)
}

@Composable
private fun PaneScreen(landscape: Boolean) = PaneScreenMobile(
    pane = PaneState(PANE_ID, StyleTable()),
    info = null,
    view = PaneView.Terminal,
    surfaces = FallbackSurfaces,
    landscape = landscape,
    readOnly = false,
    onBack = {},
    onView = {},
    onAnswer = {},
    modifier = Modifier.fillMaxSize(),
)

// The controls that sit at the very top of the pane screen. Anything further down the header
// clears a 32 dp bar whatever the code does, so asserting on it would pass either way.
private val TOPMOST = listOf("Back to the herd", "Pane actions")

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.topOf(labels: List<String>): Dp = labels
    .flatMap { onAllNodesWithContentDescription(it, substring = true).fetchSemanticsNodes() }
    .also { assertTrue(it.isNotEmpty(), "none of $labels is on this screen, so nothing was measured") }
    .minOf { with(density) { it.boundsInRoot.top.toDp() } }

// Every layout assertion in this suite measures Compose's own semantics tree, which knows nothing
// about what SystemUI paints on top of it — so a full suite passed while the gesture handle sat on
// the word "Pane". `LocalSafeArea` exists so a test can put the bars back.
@OptIn(ExperimentalTestApi::class)
class SafeAreaTest {
    @Test
    fun theBottomNavigationClearsTheGestureHandle() = runComposeUiTest {
        setContent {
            Bars {
                Column(Modifier.fillMaxSize()) {
                    Box(Modifier.weight(1f))
                    BottomNav(Tab.Herd, {})
                }
            }
        }
        // Against the window rather than a number: a fixed height taller than the test window is
        // how this assertion passed while the defect was on screen.
        val screen = onRoot().getUnclippedBoundsInRoot()
        for (tab in listOf("Herd tab", "Pane tab", "Nodes tab")) {
            val bounds = onNodeWithContentDescription(tab).getUnclippedBoundsInRoot()
            assertTrue(
                bounds.bottom <= screen.bottom - BARS.bottom,
                "$tab reaches ${bounds.bottom} of ${screen.bottom}, inside the ${BARS.bottom} the " +
                    "system draws its gesture handle in",
            )
        }
    }

    // Rotated with three-button navigation the bar leaves the bottom of the window and takes a
    // side, which is where the herd's own header sits. Every screen but the pane goes through one
    // rule; the pane is excluded because its terminal is the one surface that paints to the edge.
    @Test
    fun everyScreenButThePaneIsHeldOffTheBarsThatTakeASide() {
        for (bars in SIDE_BARS) {
            runComposeUiTest {
                setContent {
                    Bars(bars) {
                        Box(Modifier.fillMaxSize().screenInset(Screen.Herd)) {
                            HerdLandscape(Herd(), now = 0.0, localRtt = null, triage = emptyList(), onOpenPane = {}, onApprove = null)
                        }
                    }
                }
                val screen = onRoot().getUnclippedBoundsInRoot()
                val title = onNodeWithText("Herd").getUnclippedBoundsInRoot()
                assertTrue(
                    title.left >= bars.left && title.right <= screen.right - bars.right,
                    "$bars: the herd header spans ${title.left}..${title.right} of ${screen.right}",
                )
            }
        }
    }

    // The pane is the exception, and it has to stay one: its terminal paints to the edge and its
    // own chrome is what stands clear.
    @Test
    fun thePaneScreenIsNotPaddedByTheScaffoldAtAll() = runComposeUiTest {
        setContent {
            Bars {
                Box(Modifier.fillMaxSize().screenInset(Screen.Pane("x", PaneView.Terminal))) {
                    Box(Modifier.fillMaxSize().named(SHEET_FLOOR))
                }
            }
        }
        val screen = onRoot().getUnclippedBoundsInRoot()
        val inner = onNodeWithContentDescription(SHEET_FLOOR).getUnclippedBoundsInRoot()
        assertTrue(
            inner.top == screen.top && inner.bottom == screen.bottom,
            "the scaffold inset the pane to ${inner.top}..${inner.bottom} of " +
                "${screen.top}..${screen.bottom}, letterboxing the grid",
        )
    }

    // A sheet is the bottom of the window by definition, so it is where the gesture handle lands.
    @Test
    fun aSheetKeepsItsLastControlOffTheGestureHandle() {
        for (breakpoint in listOf(Breakpoint.Portrait, Breakpoint.Landscape)) {
            runComposeUiTest {
                setContent {
                    Bars {
                        BottomSheet(breakpoint, onDismiss = {}) {
                            Box(Modifier.fillMaxWidth().height(400.dp))
                            Box(Modifier.fillMaxWidth().height(48.dp).named(SHEET_FLOOR))
                        }
                    }
                }
                val screen = onRoot().getUnclippedBoundsInRoot()
                val floor = onNodeWithContentDescription(SHEET_FLOOR).getUnclippedBoundsInRoot()
                assertTrue(
                    floor.bottom <= screen.bottom - BARS.bottom,
                    "$breakpoint: the sheet's last control reaches ${floor.bottom} of " +
                        "${screen.bottom}, inside the ${BARS.bottom} gesture handle",
                )
            }
        }
    }

    // The pane screen paints edge to edge on purpose — the grid runs under the clock — but the
    // header floating over it is a control, and the clock was sitting on the pane's title.
    @Test
    fun thePaneHeaderClearsTheStatusBar() {
        for (landscape in listOf(false, true)) {
            runComposeUiTest {
                setContent { Bars { PaneScreen(landscape) } }
                val screen = onRoot().getUnclippedBoundsInRoot()
                val top = topOf(TOPMOST)
                assertTrue(
                    top >= screen.top + BARS.top,
                    "landscape=$landscape: the topmost pane control starts at $top, inside the " +
                        "${BARS.top} the system draws the status bar in",
                )
            }
        }
    }

    // Rotate the phone and the bars rotate with it. The back arrow is the leftmost thing on the
    // screen and the pane menu the rightmost, which is exactly where a rotated navigation bar and
    // a rotated cutout land.
    @Test
    fun thePaneHeaderClearsTheBarsThatMoveToTheSideInLandscape() {
        for (bars in SIDE_BARS) {
            runComposeUiTest {
                setContent { Bars(bars) { PaneScreen(landscape = true) } }
                val screen = onRoot().getUnclippedBoundsInRoot()
                val controls = TOPMOST
                    .flatMap { onAllNodesWithContentDescription(it, substring = true).fetchSemanticsNodes() }
                    .also { assertTrue(it.isNotEmpty(), "nothing was measured") }
                    .map { with(density) { it.boundsInRoot.left.toDp() to it.boundsInRoot.right.toDp() } }
                assertTrue(
                    controls.minOf { it.first } >= bars.left,
                    "$bars: a pane control starts at ${controls.minOf { it.first }}, inside the " +
                        "${bars.left} the system draws in on the left",
                )
                assertTrue(
                    controls.maxOf { it.second } <= screen.right - bars.right,
                    "$bars: a pane control reaches ${controls.maxOf { it.second }} of " +
                        "${screen.right}, inside the ${bars.right} the system draws in on the right",
                )
            }
        }
    }
}
