package dev.kampr.mosaic

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getBoundsInRoot
import androidx.compose.ui.test.hasStateDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private object TileBlank : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)
}

private const val A = "01JNODE/n:a"
private const val B = "01JNODE/n:b"
private const val C = "01JNODE/n:c"

// A | (B over C), a 40/60 split right and then an even split down: the shape the TUI's mosaic draws
// from the same paths, and the one a grid of cells drew as a 2x2 with a hole in it.
private val HERD = """{"t":"herd","nodes":[{"id":"01JNODE","name":"comingclean","kind":"local"}],"panes":[
    {"id":"$C","node_id":"01JNODE","provider":"native","tile":{"group":"01JNODE/k:a","path":[{"split":"right","ratio":400,"second":true},{"split":"down","ratio":500,"second":true}]}},
    {"id":"$A","node_id":"01JNODE","provider":"native","tile":{"group":"01JNODE/k:a","path":[{"split":"right","ratio":400}]}},
    {"id":"$B","node_id":"01JNODE","provider":"native","tile":{"group":"01JNODE/k:a","path":[{"split":"right","ratio":400,"second":true},{"split":"down","ratio":500}]}}
]}"""

private fun deskTokens() = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    .let { KamprTokens(SoftTheme, it, typography(it, SoftTheme.label, TypeScale.Desk)) }

@OptIn(ExperimentalTestApi::class)
class MosaicTileTest {
    @Test
    fun aTabOpenedSideBySideIsDrawnAsItsSplitTree() = runComposeUiTest {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val store = KamprStore()
        val state = AppState(scope, store, MemoryPrefs(), null)
        val host = MosaicSurfaces()
        try {
            store.accept(Wire.decode(HERD) ?: error("undecodable"))
            state.openPane(B)
            state.openTab(B)
            assertEquals(Screen.Mosaic, state.screen)
            setContent {
                CompositionLocalProvider(LocalTokens provides deskTokens(), LocalPaneIo provides ArtboardIo) {
                    Box(Modifier.size(1000.dp, 800.dp)) {
                        host.Mosaic(state, Breakpoint.Desktop, TileBlank, Modifier)
                    }
                }
            }
            waitForIdle()
            assertEquals(listOf(A, B, C), host.held?.panes, "the tab, in the tree's order")
            val a = onNode(hasStateDescription("cell 1 of 3")).getBoundsInRoot()
            val b = onNode(hasStateDescription("cell 2 of 3")).getBoundsInRoot()
            val c = onNode(hasStateDescription("cell 3 of 3")).getBoundsInRoot()
            assertEquals(a.top.value, b.top.value, 1f, "A and B both start at the top")
            assertTrue(c.top.value > b.top.value + 200, "C is below B, in the bottom half")
            assertEquals(b.left.value, c.left.value, 1f, "B and C share a column")
            assertTrue(abs(b.left.value - 400f) < 8f, "the 40/60 split puts the column at 400: ${b.left.value}")
            assertTrue(a.left.value < 10f, "A is the left-hand column: ${a.left.value}")
        } finally {
            scope.cancel()
        }
    }
}
