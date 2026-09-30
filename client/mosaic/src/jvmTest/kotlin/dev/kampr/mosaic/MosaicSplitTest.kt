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
import dev.kampr.shared.wire.SplitDirection
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val SOURCE = "01JNODE/w1:p1"
private const val MADE = "01JNODE/w1:p2"
private const val ELSEWHERE = "01JNODE/w2:p1"

private object NoSurface : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)
}

private fun deskTokens() = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    .let { KamprTokens(SoftTheme, it, typography(it, SoftTheme.label, TypeScale.Desk)) }

private fun herdOf(vararg ids: String) =
    """{"t":"herd","nodes":[{"id":"01JNODE","name":"comingclean","kind":"local"}],"panes":[""" +
        ids.joinToString(",") { """{"id":"$it","node_id":"01JNODE"}""" } + "]}"

private fun KamprStore.take(frame: String) = accept(Wire.decode(frame) ?: error("undecodable: $frame"))

// A split is two panes of one tab, and the only place Kampr shows two panes at once is the mosaic.
// Whatever arrangement was there before, the one that opens is exactly the split: the pane it was
// made from, then the pane it made, in the shape herdr gave them.
@OptIn(ExperimentalTestApi::class)
class MosaicSplitTest {
    private fun opened(direction: SplitDirection, check: (MosaicSurfaces, cell1: Pair<Float, Float>, cell2: Pair<Float, Float>) -> Unit) =
        runComposeUiTest {
            val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
            val store = KamprStore()
            val prefs = MemoryPrefs().apply { set("mosaic.panes", ELSEWHERE) }
            val state = AppState(scope, store, prefs, null)
            val host = MosaicSurfaces()
            try {
                store.take(herdOf(SOURCE, ELSEWHERE))
                state.openPane(SOURCE)
                state.openingSplit(SOURCE, MADE, direction)
                store.take(herdOf(SOURCE, MADE, ELSEWHERE))
                assertEquals(Screen.Mosaic, state.screen)

                setContent {
                    CompositionLocalProvider(LocalTokens provides deskTokens(), LocalPaneIo provides ArtboardIo) {
                        Box(Modifier.size(1200.dp, 800.dp)) {
                            host.Mosaic(state, Breakpoint.Desktop, NoSurface, Modifier)
                        }
                    }
                }
                waitForIdle()
                val first = onNode(hasStateDescription("cell 1 of 2")).getBoundsInRoot()
                val second = onNode(hasStateDescription("cell 2 of 2")).getBoundsInRoot()
                assertNull(state.mosaicSeed, "a seed is applied once")
                check(host, first.left.value to first.top.value, second.left.value to second.top.value)
            } finally {
                scope.cancel()
            }
        }

    @Test
    fun aSplitToTheRightOpensTheTwoPanesSideBySide() = opened(SplitDirection.Right) { host, first, second ->
        assertEquals(listOf(SOURCE, MADE), host.held?.panes)
        assertEquals(first.second, second.second, "side by side shares a top edge")
        assertTrue(second.first > first.first, "the new pane is to the right")
    }

    @Test
    fun aSplitDownwardsOpensTheTwoPanesStacked() = opened(SplitDirection.Down) { host, first, second ->
        assertEquals(listOf(SOURCE, MADE), host.held?.panes)
        assertEquals(first.first, second.first, "stacked shares a left edge")
        assertTrue(second.second > first.second, "the new pane is below")
    }
}
