package dev.kampr.mosaic

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.unreadDone
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.push.NoPush
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals

private const val FIRST = "01JNODE/w1:p1"
private const val SECOND = "01JNODE/w2:p1"

private fun finished(vararg ids: String) = """{"t":"herd","nodes":[{"id":"01JNODE","name":"box","kind":"local"}],"panes":[""" +
    ids.joinToString(",") { """{"id":"$it","node_id":"01JNODE","agent":"claude","agent_status":"done","updated_at":"7"}""" } +
    "]}"

// A finish in a cell on the screen has been seen, and the phone's notification for it has to come
// down — but only for a cell that is showing, and a phone's mosaic shows one at a time.
@OptIn(ExperimentalTestApi::class)
class MosaicReadTest {
    private fun unreadAfterShowing(breakpoint: Breakpoint): List<String> {
        val scope = CoroutineScope(SupervisorJob())
        val prefs = MemoryPrefs().apply { set("mosaic.panes", encodeArrangement(listOf(FIRST, SECOND))) }
        val state = AppState(scope, KamprStore(), prefs, null, NoPush())
        state.store.accept(Wire.decode(finished(FIRST, SECOND)) ?: error("undecodable"))
        state.go(Screen.Mosaic)
        try {
            runComposeUiTest {
                setContent {
                    CompositionLocalProvider(LocalTokens provides watchTokens(), LocalPaneIo provides ArtboardIo) {
                        MosaicSurfaces().Mosaic(state, breakpoint, WatchSurfaces, Modifier)
                    }
                }
                waitForIdle()
            }
            return state.store.herd.value.unreadDone(state.seenDone).map { it.id }
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun everyCellOnADesktopMosaicIsRead() = assertEquals(emptyList(), unreadAfterShowing(Breakpoint.Desktop))

    @Test
    fun onAPhoneOnlyTheCellShowingIsRead() {
        for (breakpoint in listOf(Breakpoint.Portrait, Breakpoint.Landscape)) {
            assertEquals(listOf(SECOND), unreadAfterShowing(breakpoint), "$breakpoint")
        }
    }
}
