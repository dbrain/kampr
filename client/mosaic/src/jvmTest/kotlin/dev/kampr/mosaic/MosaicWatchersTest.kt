package dev.kampr.mosaic

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.model.WATCH_RISE_MS
import dev.kampr.shared.model.watchersNotice
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
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

private const val SETTLE = 400L

private val WATCH_NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local", online = true)

private fun watchPane(id: String, watchers: Int?) = PaneInfo(
    id = "01JNODE/$id",
    nodeId = "01JNODE",
    workspace = "kampr",
    agent = "claude",
    agentStatus = "working",
    cols = 74,
    rows = 30,
    watchers = watchers,
)

private object WatchSurfaces : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)
}

private fun watchTokens() = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    .let { KamprTokens(SoftTheme, it, typography(it, SoftTheme.label, TypeScale.Phone)) }

// A mosaic is four panes at once. The header of every cell carries the fact, because it is one
// more thing about a pane the header already describes — but only the cell input actually reaches
// puts up the notice, or opening a mosaic of shared panes is four interruptions at once.
@OptIn(ExperimentalTestApi::class)
class MosaicWatchersTest {
    @Test
    fun onlyTheFocusedCellInterruptsAndEveryHeaderStillSaysIt() = runComposeUiTest {
        mainClock.autoAdvance = false
        val shared = watchPane("w1:p1", 3)
        val other = watchPane("w2:p1", 3)
        setContent {
            CompositionLocalProvider(LocalTokens provides watchTokens(), LocalPaneIo provides ArtboardIo) {
                Row {
                    MosaicCell(
                        PaneState(shared.id, StyleTable()), shared, WATCH_NODE, focused = true,
                        surfaces = WatchSurfaces, onFocus = {}, onRemove = {},
                        modifier = Modifier.size(420.dp, 300.dp),
                    )
                    MosaicCell(
                        PaneState(other.id, StyleTable()), other, WATCH_NODE, focused = false,
                        surfaces = WatchSurfaces, onFocus = {}, onRemove = {},
                        modifier = Modifier.size(420.dp, 300.dp),
                    )
                }
            }
        }
        mainClock.advanceTimeBy(WATCH_RISE_MS + SETTLE)
        waitForIdle()

        assertEquals(
            2,
            onAllNodesWithText("ALSO OPEN · 2", substring = true).fetchSemanticsNodes().size,
            "both headers should carry the fact, focused or not",
        )
        assertEquals(
            1,
            onAllNodesWithContentDescription(assertNotNull(watchersNotice(2))).fetchSemanticsNodes().size,
            "a mosaic of shared panes must interrupt once, for the cell being typed into",
        )
    }

    @Test
    fun aCellNobodyElseHasOpenSaysNothing() = runComposeUiTest {
        mainClock.autoAdvance = false
        val alone = watchPane("w1:p1", null)
        setContent {
            CompositionLocalProvider(LocalTokens provides watchTokens(), LocalPaneIo provides ArtboardIo) {
                MosaicCell(
                    PaneState(alone.id, StyleTable()), alone, WATCH_NODE, focused = true,
                    surfaces = WatchSurfaces, onFocus = {}, onRemove = {},
                    modifier = Modifier.size(420.dp, 300.dp),
                )
            }
        }
        mainClock.advanceTimeBy(WATCH_RISE_MS + SETTLE)
        waitForIdle()
        assertTrue(
            onAllNodesWithText("ALSO OPEN", substring = true).fetchSemanticsNodes().isEmpty(),
            "a cell nobody else had open said somebody did",
        )
    }
}
