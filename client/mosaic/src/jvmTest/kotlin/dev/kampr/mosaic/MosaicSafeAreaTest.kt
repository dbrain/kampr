package dev.kampr.mosaic

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
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
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.terminal.TerminalSurfaces
import kotlin.test.Test
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

// A gesture handle's worth of system furniture, and a status bar's — the API 37 AVD's own numbers.
private val BARS = SafeArea(top = 32.dp, bottom = 46.dp)

private fun testTokens() = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    .let { KamprTokens(SoftTheme, it, typography(it, SoftTheme.label, TypeScale.Phone)) }

// The real terminal underneath, with the one number it is told about the chrome above it read back
// out. A stub surface would prove the bar moved and nothing about the grid the bar sits on.
private class ChromeProbe(private val inner: PaneSurfaces = TerminalSurfaces()) : PaneSurfaces {
    var top: Dp? = null

    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) {
        top = LocalPaneChrome.current?.top
        inner.Terminal(pane, info, modifier)
    }

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) =
        inner.Conversation(pane, info, modifier)

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) =
        inner.KeyRow(pane, compact, modifier)

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) = inner.Zoom(pane, modifier)
}

// The switcher's own bar: the back arrow and the trailing cluster sit at the very top of it, and
// the pane chips at the bottom. Anything between the two clears a 32 dp bar either way.
private val BAR_CONTROLS = listOf(
    "Back to the herd",
    "Add a pane to the mosaic",
    "Remove ",
    "Show ",
)

// The caps on the bottom row of the key row, in both layouts.
private val LAST_ROW = listOf("End", "Left arrow", "Down arrow", "Right arrow")

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.boundsOf(labels: List<String>) = labels
    .flatMap { onAllNodesWithContentDescription(it, substring = true).fetchSemanticsNodes() }
    .map { it.boundsInRoot }
    .also { assertTrue(it.isNotEmpty(), "none of $labels is on this screen, so nothing was measured") }

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.switcher(landscape: Boolean, probe: ChromeProbe) {
    val fixture = Fixture().apply { fourPanes() }
    setContent {
        CompositionLocalProvider(
            LocalTokens provides testTokens(),
            LocalPaneIo provides ArtboardIo,
            LocalSafeArea provides BARS,
        ) {
            Box(Modifier.fillMaxSize()) {
                MosaicSwitcher(
                    store = fixture.store,
                    mosaic = fixture.mosaic,
                    herd = fixture.store.herd.value,
                    surfaces = probe,
                    landscape = landscape,
                    onHerd = {},
                    onAdd = {},
                )
            }
        }
    }
    waitForIdle()
}

// The switcher is the mosaic on a phone and it wears no bottom navigation, so nothing else was
// holding its key row off the gesture handle and nothing at all was holding its bar off the clock.
@OptIn(ExperimentalTestApi::class)
class MosaicSafeAreaTest {
    @Test
    fun theSwitcherBarClearsTheStatusBarAndTheTerminalKnowsItMoved() {
        for (landscape in listOf(false, true)) {
            runComposeUiTest {
                val probe = ChromeProbe()
                switcher(landscape, probe)
                val bar = boundsOf(BAR_CONTROLS)
                assertTrue(
                    with(density) { bar.minOf { it.top }.toDp() } >= BARS.top,
                    "landscape=$landscape: the switcher's bar starts at " +
                        "${with(density) { bar.minOf { it.top }.toDp() }}, inside the ${BARS.top} " +
                        "the system draws the status bar in",
                )
                // The bar floats over the grid and the grid insets its scrollable content by
                // whatever the bar takes. Moving the bar down without telling the terminal hides
                // a row behind it with no scroll left to reach it.
                val inset = assertNotNull(probe.top, "the terminal was never told what the bar takes")
                assertTrue(
                    with(density) { inset.toPx() } >= bar.maxOf { it.bottom },
                    "landscape=$landscape: the terminal insets $inset but the bar reaches " +
                        "${with(density) { bar.maxOf { it.bottom }.toDp() }}",
                )
            }
        }
    }

    // The desktop mosaic stacks its bars rather than floating them, and a tablet has the same two
    // strips of system furniture a phone does.
    @Test
    fun theDesktopMosaicBarsClearBothSystemBars() = runComposeUiTest {
        val fixture = Fixture().apply { fourPanes() }
        setContent {
            CompositionLocalProvider(
                LocalTokens provides testTokens(),
                LocalPaneIo provides ArtboardIo,
                LocalSafeArea provides BARS,
            ) {
                Box(Modifier.fillMaxSize()) {
                    MosaicScreen(
                        store = fixture.store,
                        mosaic = fixture.mosaic,
                        herd = fixture.store.herd.value,
                        connectionStatus = ConnectionStatus.Live("full"),
                        build = "0.1.0",
                        surfaces = TerminalSurfaces(),
                        onHerd = {},
                        onAdd = {},
                    )
                }
            }
        }
        waitForIdle()
        val screen = onRoot().getUnclippedBoundsInRoot()
        val top = with(density) { boundsOf(listOf("Add pane", "Save layout", "Saved on this device")).minOf { it.top }.toDp() }
        assertTrue(top >= BARS.top, "the mosaic bar starts at $top, inside the ${BARS.top} status bar")
        val bottom = with(density) {
            boundsOf(listOf("observe streams open")).maxOf { it.bottom }.toDp()
        }
        assertTrue(
            bottom <= screen.bottom - BARS.bottom,
            "the mosaic status row reaches $bottom of ${screen.bottom}, inside the ${BARS.bottom} " +
                "the system draws its gesture handle in",
        )
    }

    @Test
    fun theSwitcherKeyRowClearsTheGestureHandle() {
        for (landscape in listOf(false, true)) {
            runComposeUiTest {
                switcher(landscape, ChromeProbe())
                val screen = onRoot().getUnclippedBoundsInRoot()
                val lowest = with(density) { boundsOf(LAST_ROW).maxOf { it.bottom }.toDp() }
                assertTrue(
                    lowest <= screen.bottom - BARS.bottom,
                    "landscape=$landscape: the key row reaches $lowest of ${screen.bottom}, " +
                        "inside the ${BARS.bottom} the system draws its gesture handle in",
                )
            }
        }
    }
}
