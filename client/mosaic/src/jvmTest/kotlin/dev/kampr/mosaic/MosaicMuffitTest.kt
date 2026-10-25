package dev.kampr.mosaic

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
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.named
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals

private const val TERMINAL = "Terminal probe"
private const val CONVERSATION = "Conversation probe"
private const val KEYS = "Key row probe"
private const val DETAIL = "No pane on this node can show a screen: Kampr cannot run herdr."

private val NODE = NodeInfo(id = HUB, name = "comingclean", kind = "local", online = false, detail = "no herdr", reachable = true, native = true)

private val MUFFIT = PaneInfo(
    id = "$HUB/m:s1",
    nodeId = HUB,
    label = "tidy the parser",
    agent = "muffit",
    provider = "muffit",
    agentStatus = "working",
    rows = 0,
    hasConversation = true,
    converses = true,
    detail = DETAIL,
)

private object Probe : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier.named(TERMINAL))

    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier.named(CONVERSATION))

    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier.size(40.dp).named(KEYS))
}

private fun tokens() = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    .let { KamprTokens(SoftTheme, it, typography(it, SoftTheme.label, TypeScale.Phone)) }

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.count(label: String) = onAllNodesWithContentDescription(label, substring = true).fetchSemanticsNodes().size

// docs/18: a muffit session has no grid, so wherever the mosaic shows one it shows its
// conversation, and a cell that never paints is not a stream that failed.
@OptIn(ExperimentalTestApi::class)
class MosaicMuffitTest {
    @Test
    fun aCellShowsAMuffitSessionsConversation() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides tokens(), LocalPaneIo provides ArtboardIo) {
                MosaicCell(
                    PaneState(MUFFIT.id, StyleTable()),
                    MUFFIT,
                    NODE,
                    focused = true,
                    surfaces = Probe,
                    onFocus = {},
                    onRemove = {},
                    modifier = Modifier.size(420.dp, 300.dp),
                )
            }
        }
        waitForIdle()
        onNodeWithContentDescription(CONVERSATION).assertExists()
        assertEquals(0, count(TERMINAL), "a muffit cell drew a terminal")
        assertEquals(0, count(DETAIL), "a muffit cell was said to have failed to paint")
        assertEquals(0, count("is unavailable"), "a muffit cell was washed over for its herdr")
    }

    @Test
    fun theSwitcherShowsAMuffitSessionsConversationWithNoKeys() = runComposeUiTest {
        val fixture = Fixture()
        fixture.store.accept(ServerMsg.Herd(listOf(NODE), listOf(MUFFIT)))
        fixture.mosaic.add(MUFFIT.id)
        fixture.mosaic.focus(MUFFIT.id)
        setContent {
            CompositionLocalProvider(LocalTokens provides tokens(), LocalPaneIo provides ArtboardIo) {
                Box(Modifier.fillMaxSize()) {
                    MosaicSwitcher(
                        store = fixture.store,
                        mosaic = fixture.mosaic,
                        herd = fixture.store.herd.value,
                        surfaces = Probe,
                        landscape = false,
                        onHerd = {},
                        onAdd = {},
                    )
                }
            }
        }
        waitForIdle()
        onNodeWithContentDescription(CONVERSATION).assertExists()
        assertEquals(0, count(TERMINAL), "the switcher drew a terminal for a muffit session")
        assertEquals(0, count(KEYS), "the switcher laid a key row over a muffit session")
    }
}
