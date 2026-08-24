package dev.kampr.mosaic

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
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
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals

private val NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local", online = true)

private const val DETAIL = "No pane on this node can show a screen: Kampr cannot run herdr."

private fun cellPane(detail: String?) = PaneInfo(
    id = "01JNODE/w1:p1",
    nodeId = "01JNODE",
    workspace = "kampr",
    agentStatus = "idle",
    cols = 74,
    rows = 30,
    detail = detail,
)

private object Blank : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)
}

private fun cellTokens() = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    .let { KamprTokens(SoftTheme, it, typography(it, SoftTheme.label, TypeScale.Phone)) }

// A mosaic of four blank cells is where this defect is worst: nothing on the screen moves and
// nothing on it says why. An online node with no way to stream needs the same card the mesh
// already puts up for an offline one.
@OptIn(ExperimentalTestApi::class)
class MosaicStreamFaultTest {
    @Test
    fun anOnlineNodeThatCannotStreamStillSaysSoInTheCell() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides cellTokens(), LocalPaneIo provides ArtboardIo) {
                val info = cellPane(DETAIL)
                MosaicCell(
                    PaneState(info.id, StyleTable()), info, NODE, focused = true,
                    surfaces = Blank, onFocus = {}, onRemove = {},
                    modifier = Modifier.size(420.dp, 300.dp),
                )
            }
        }
        waitForIdle()
        onNodeWithContentDescription(DETAIL, substring = true).assertExists()
    }

    @Test
    fun aCellWithNothingWrongWithItStaysAGrid() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides cellTokens(), LocalPaneIo provides ArtboardIo) {
                val info = cellPane(null)
                MosaicCell(
                    PaneState(info.id, StyleTable()), info, NODE, focused = true,
                    surfaces = Blank, onFocus = {}, onRemove = {},
                    modifier = Modifier.size(420.dp, 300.dp),
                )
            }
        }
        waitForIdle()
        assertEquals(
            0,
            onAllNodesWithContentDescription("has no picture", substring = true).fetchSemanticsNodes().size,
        )
    }
}
