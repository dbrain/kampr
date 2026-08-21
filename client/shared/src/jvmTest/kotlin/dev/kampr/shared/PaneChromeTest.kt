package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
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
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.named
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

private const val PANE_ID = "01JNODE/w1:p1"

private val INFO = PaneInfo(
    id = PANE_ID,
    nodeId = "01JNODE",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = "idle",
    cols = 94,
    rows = 40,
    hasConversation = true,
)

private const val ZOOM_PROBE = "Zoom probe"

private class ChromeProbe : PaneSurfaces {
    var chrome: Dp? = null

    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) {
        chrome = LocalPaneChrome.current?.top
        Box(modifier)
    }

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) =
        Box(modifier.size(40.dp).named(ZOOM_PROBE))
}

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), content = content)
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.screen(
    probe: ChromeProbe,
    width: Dp,
    height: Dp,
    landscape: Boolean,
    view: PaneView,
) {
    setContent {
        Themed {
            Box(Modifier.size(width, height)) {
                PaneScreenMobile(
                    pane = PaneState(PANE_ID, StyleTable()),
                    info = INFO,
                    view = view,
                    surfaces = probe,
                    landscape = landscape,
                    readOnly = false,
                    onBack = {},
                    onView = {},
                    onAnswer = {},
                    modifier = Modifier.fillMaxSize(),
                )
            }
        }
    }
    waitForIdle()
}

// The header floats over the terminal and the terminal insets its scrollable content by whatever
// the header takes. A constant guessed at 108 dp against 121 dp of real bar hides the top row
// behind it with no scroll left to recover it, so the number has to come off the layout.
@OptIn(ExperimentalTestApi::class)
class PaneChromeTest {
    @Test
    fun theTerminalIsInsetByTheChromeThatIsActuallyDrawnPortrait() = runComposeUiTest {
        val probe = ChromeProbe()
        screen(probe, 411.dp, 914.dp, landscape = false, view = PaneView.Terminal)
        assertChromeCovered(probe)
    }

    @Test
    fun theTerminalIsInsetByTheChromeThatIsActuallyDrawnLandscape() = runComposeUiTest {
        val probe = ChromeProbe()
        screen(probe, 914.dp, 411.dp, landscape = true, view = PaneView.Terminal)
        assertChromeCovered(probe)
    }

    // Landscape is a first-class layout, and an agent pane opens in Conversation: with the switcher
    // hidden there is no way to reach the terminal at all without rotating the phone.
    @Test
    fun landscapeCanStillReachTheTerminalAndTheZoom() = runComposeUiTest {
        val probe = ChromeProbe()
        screen(probe, 914.dp, 411.dp, landscape = true, view = PaneView.Conversation)
        onNodeWithContentDescription("Terminal view").assertExists()
        onNodeWithContentDescription("Conversation view").assertExists()
        onNodeWithContentDescription(ZOOM_PROBE).assertExists()
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.assertChromeCovered(probe: ChromeProbe) {
    val inset = assertNotNull(probe.chrome, "the terminal was never told what the chrome takes")
    val bottom = listOf("Back to the herd", "Terminal view", "Conversation view", ZOOM_PROBE)
        .flatMap { onAllNodesWithContentDescription(it).fetchSemanticsNodes() }
        .maxOf { it.boundsInRoot.bottom }
    val insetPx = with(density) { inset.toPx() }
    assertTrue(
        insetPx >= bottom,
        "the terminal insets $insetPx px but the chrome reaches $bottom px, so the top row is under it",
    )
}
