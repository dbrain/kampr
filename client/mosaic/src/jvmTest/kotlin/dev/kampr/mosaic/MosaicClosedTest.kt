package dev.kampr.mosaic

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneGone
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

private const val PANE = "01JNODEABCDEFGHJKMNPQRSTV/w1:p1"

private val HOST = NodeInfo(id = "01JNODEABCDEFGHJKMNPQRSTV", name = "comingclean", kind = "local")

private object BlankCell : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)
}

private fun tokens() = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    .let { KamprTokens(SoftTheme, it, typography(it, SoftTheme.label, TypeScale.Phone)) }

// The same defect as the pane screen's, one surface over: a shell exits, its herd entry goes, and
// the cell that was watching it puts a node ULID where its name was. Four cells at once is where
// it reads worst, because nothing else on the screen moves either.
@OptIn(ExperimentalTestApi::class)
class MosaicClosedTest {
    @Test
    fun aClosedCellSaysClosedRatherThanShowingItsId() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides tokens(), LocalPaneIo provides ArtboardIo) {
                MosaicCell(
                    PaneState(PANE, StyleTable()), null, HOST, focused = true,
                    surfaces = BlankCell, onFocus = {}, onRemove = {},
                    gone = PaneGone.Shell,
                    modifier = Modifier.size(420.dp, 300.dp),
                )
            }
        }
        waitForIdle()
        onNodeWithText("CLOSED").assertExists()
        assertEquals(
            0,
            onAllNodesWithText(PANE, substring = true).fetchSemanticsNodes().size,
            "the cell header fell back to the raw pane id",
        )
    }

    @Test
    fun aLiveCellWearsNoSuchMark() = runComposeUiTest {
        val info = PaneInfo(id = PANE, nodeId = HOST.id, workspace = "kampr", agentStatus = "idle", cols = 74, rows = 30)
        setContent {
            CompositionLocalProvider(LocalTokens provides tokens(), LocalPaneIo provides ArtboardIo) {
                MosaicCell(
                    PaneState(PANE, StyleTable()), info, HOST, focused = true,
                    surfaces = BlankCell, onFocus = {}, onRemove = {},
                    modifier = Modifier.size(420.dp, 300.dp),
                )
            }
        }
        waitForIdle()
        assertEquals(0, onAllNodesWithText("CLOSED").fetchSemanticsNodes().size)
        onNodeWithText("kampr · bash").assertExists()
    }
}
