package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.ui.AppScaffold
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.AuthSurface
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.NoMosaic
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "01JHERE/w1:p1"

private const val HERD = """
    {"t":"herd",
     "nodes":[{"id":"01JHERE","name":"comingclean","kind":"local","online":true},
              {"id":"01JTHERE","name":"workbox","kind":"peer","online":true}],
     "panes":[
       {"id":"01JHERE/w1:p1","node_id":"01JHERE","workspace":"kampr","tab":"1",
        "cwd":"/home/dbrain/dev/kampr","agent":"claude","agent_status":"idle",
        "cols":94,"rows":40,"has_conversation":true}]}
"""

private val PHONE = DpSize(390.dp, 844.dp)
private val ROTATED = DpSize(844.dp, 390.dp)
private val DESK = DpSize(1440.dp, 900.dp)

private val TABS = listOf("Herd tab", "Settings tab")

// What the pane's key row is told it owes at the bottom of the window, which is the gesture handle
// when nothing of the app's own is under it and nothing when something is.
private class OwedSurfaces : PaneSurfaces {
    var owed: Dp = Dp.Unspecified

    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) {
        owed = LocalSafeArea.current.bottom
        Box(modifier)
    }

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) = Box(modifier.size(40.dp))
}

private fun app(): Pair<AppState, CoroutineScope> {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
    val store = KamprStore()
    store.accept(Wire.decode(HERD) ?: error("undecodable herd"))
    val prefs = MemoryPrefs().apply {
        set("endpoint", "http://127.0.0.1:8790")
        set("token", "kmp_stored")
    }
    return AppState(scope, store, prefs, null) to scope
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.window(
    state: AppState,
    breakpoint: Breakpoint,
    size: DpSize,
    surfaces: PaneSurfaces,
    connection: ConnectionStatus = ConnectionStatus.Live("full"),
) {
    setContent {
        Bars {
            Box(Modifier.size(size)) {
                AppScaffold(
                    state = state,
                    breakpoint = breakpoint,
                    surfaces = surfaces,
                    mosaic = NoMosaic,
                    now = 1_787_000_000_000.0,
                    auth = AuthSurface(null, emptyList(), null, null, null, {}, {}, {}, {}),
                    connectionStatus = connection,
                    deepLink = null,
                )
            }
        }
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.tabs(): Int = TABS.sumOf { onAllNodesWithContentDescription(it).fetchSemanticsNodes().size }

// The operator, on a phone: "Don't show the bottom sheet on Android/mobile when in a session — just
// on the home screen, i.e. open session → press back → then can see Herd/Settings." And on the
// desk: "Drop the bottom bar on wasm desktop."
@OptIn(ExperimentalTestApi::class)
class TheBottomOfTheWindowTest {
    // Both postures, because portrait wore the tabs under a pane long after landscape stopped. With
    // nothing of the app's own under it, the pane is what ends at the window and owes the handle.
    @Test
    fun aPaneOnAPhoneHasNoTabsUnderItAndBackBringsThemBack() {
        for ((breakpoint, size) in listOf(Breakpoint.Portrait to PHONE, Breakpoint.Landscape to ROTATED)) {
            runComposeUiTest {
                val (state, scope) = app()
                try {
                    val surfaces = OwedSurfaces()
                    window(state, breakpoint, size, surfaces)
                    assertEquals(TABS.size, tabs(), "$breakpoint: the herd has no tabs to leave by")

                    state.openPane(PANE, PaneView.Terminal)
                    waitForIdle()
                    assertEquals(0, tabs(), "$breakpoint: a pane is wearing the tab bar")
                    assertEquals(BARS.bottom, surfaces.owed, "$breakpoint: nothing under the pane, and it was told something was")

                    state.back()
                    waitForIdle()
                    assertEquals(Screen.Herd, state.screen, "$breakpoint: back from a pane went somewhere other than the herd")
                    assertEquals(TABS.size, tabs(), "$breakpoint: back to the herd and the tabs did not come with it")
                } finally {
                    scope.cancel()
                }
            }
        }
    }

    // What the strip said lives in the sidebar, and a held pane says so in its own header.
    @Test
    fun theDeskEndsInWhateverScreenIsOpenAndNotInAStatusStrip() {
        for (open in listOf(false, true)) {
            runComposeUiTest {
                val (state, scope) = app()
                try {
                    if (open) state.openPane(PANE, PaneView.Terminal)
                    val surfaces = OwedSurfaces()
                    window(state, Breakpoint.Desktop, DESK, surfaces)
                    val strip = listOf("hub · ", "desktop shape untouched", "showing cached grid")
                        .sumOf { onAllNodesWithText(it, substring = true).fetchSemanticsNodes().size } +
                        onAllNodesWithContentDescription("to hub", substring = true).fetchSemanticsNodes().size
                    assertEquals(0, strip, "open=$open: the desk still ends in a status strip")
                    if (open) {
                        assertEquals(BARS.bottom, surfaces.owed, "a desk pane was told something is under it")
                    }
                    assertTrue(tabs() == 0, "open=$open: a desk wears phone tabs")
                } finally {
                    scope.cancel()
                }
            }
        }
    }

    // The strip was where a dropped socket was said with the sidebar folded away. The expanded
    // sidebar's machine pill says it; the rail has to as well.
    @Test
    fun aFoldedSidebarStillSaysTheSocketDropped() {
        for (collapsed in listOf(false, true)) {
            runComposeUiTest {
                val (state, scope) = app()
                try {
                    state.collapseSidebar(collapsed)
                    window(state, Breakpoint.Desktop, DESK, OwedSurfaces(), ConnectionStatus.Offline("the wifi went", 5_000))
                    assertTrue(
                        onAllNodesWithContentDescription("Reconnecting", substring = true).fetchSemanticsNodes().isNotEmpty(),
                        "collapsed=$collapsed: the desk says nothing about a socket that has gone",
                    )
                } finally {
                    scope.cancel()
                }
            }
        }
    }
}
