package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.model.WATCH_NOTICE_MS
import dev.kampr.shared.model.WATCH_RISE_MS
import dev.kampr.shared.model.watchersNotice
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

private val NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local", online = true)

private fun pane(id: String, watchers: Int?) = PaneInfo(
    id = "01JNODE/$id",
    nodeId = "01JNODE",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = "working",
    cols = 94,
    rows = 40,
    watchers = watchers,
)

// One tick of the settle loop, plus a frame to notice it.
private const val SETTLE = 400L

private val ALONE = pane("w1:p1", null)
private val SHARED = pane("w2:p1", 2)

private class WatchProbe : PaneSurfaces {
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
class WatchersUiTest {
    // The herd list already carries status per pane; this is one more fact about the same row,
    // and it is spoken as part of the row rather than announced, because a herd of forty panes
    // announcing every join is chatter, not information.
    @Test
    fun theHerdListSaysWhichPaneAnotherClientHasOpenAndOnlyThatOne() = runComposeUiTest {
        setContent {
            Themed {
                HerdPortrait(
                    Herd(nodes = listOf(NODE), panes = listOf(ALONE, SHARED), known = true),
                    ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null,
                )
            }
        }
        waitForIdle()
        onNodeWithContentDescription("also open on another client", substring = true).assertExists()
        assertEquals(
            1,
            onAllNodesWithContentDescription("also open", substring = true).fetchSemanticsNodes().size,
            "a pane nobody else has open claimed somebody did",
        )
    }

    // The one the brief calls the important one, and the one most easily made annoying. It floats
    // over the grid: the terminal insets its scrollable content by the chrome it measures, so a
    // notice that joined the chrome would hide a row of the pane behind the thing announcing it.
    @Test
    fun thePaneNoticeAppearsOverTheGridThenGoesWithoutCostingARow() = runComposeUiTest {
        // The notice is a transient, and an auto-advancing clock winds straight past it.
        mainClock.autoAdvance = false
        val probe = WatchProbe()
        var info by mutableStateOf(ALONE)
        setContent {
            Themed {
                Box(Modifier.size(411.dp, 914.dp)) {
                    PaneScreenMobile(
                        pane = PaneState(ALONE.id, StyleTable()),
                        info = info,
                        view = PaneView.Terminal,
                        surfaces = probe,
                        landscape = false,
                        readOnly = false,
                        onBack = {},
                        onView = {},
                    )
                }
            }
        }
        mainClock.advanceTimeBy(SETTLE)
        waitForIdle()
        val quiet = assertNotNull(probe.chrome, "the terminal was never handed a chrome height")
        assertTrue(
            onAllNodesWithText("also open", substring = true).fetchSemanticsNodes().isEmpty(),
            "a pane nobody else had open said somebody did",
        )

        info = ALONE.copy(watchers = 2)
        val notice = assertNotNull(watchersNotice(1))
        mainClock.advanceTimeBy(WATCH_RISE_MS + SETTLE)
        waitForIdle()
        onNodeWithContentDescription(notice)
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Polite))
        assertEquals(quiet, probe.chrome, "the notice grew the chrome, which costs the pane a row")

        mainClock.advanceTimeBy(WATCH_NOTICE_MS + SETTLE)
        waitForIdle()
        assertTrue(
            onAllNodesWithContentDescription(notice).fetchSemanticsNodes().isEmpty(),
            "the notice never left, which is the permanent badge this must not be",
        )
        assertEquals(quiet, probe.chrome, "the notice left the chrome a different size than it found it")
        assertTrue(
            onAllNodesWithText("also open", substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the fact went away with the notice, so there is no way to check it afterwards",
        )
    }
}
