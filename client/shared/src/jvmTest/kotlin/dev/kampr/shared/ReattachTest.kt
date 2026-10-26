package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.InternalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.backhandler.LocalCompatNavigationEventDispatcherOwner
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.push.NoPush
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppScaffold
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.AuthSurface
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.NoMosaic
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val FLEET_RUN = "01JNODE/fleet:r1"
private const val SHELL = "01JNODE/w1:p1"
private const val AGENT = "01JNODE/w2:p1"
private const val BLOCKED = "01JNODE/w3:p1"

private fun herd(vararg panes: String) = """{"t":"herd","nodes":[{"id":"01JNODE","name":"box","kind":"local","online":true}],""" +
    """"panes":[${panes.joinToString(",")}]}"""

private val FLEET_PANE = """{"id":"$FLEET_RUN","node_id":"01JNODE","agent_status":"unknown",""" +
    """"fleet":{"cohort":"c1","command":"apt upgrade","state":"exited","exit_code":0},"updated_at":"2026-10-26T09:00:00Z"}"""
private val SHELL_PANE = """{"id":"$SHELL","node_id":"01JNODE","workspace":"shell","agent_status":"unknown","updated_at":"2026-10-26T08:01:00Z"}"""
private val AGENT_PANE = """{"id":"$AGENT","node_id":"01JNODE","workspace":"kampr","agent":"claude","agent_status":"idle","updated_at":"2026-10-26T08:02:00Z"}"""
private val BLOCKED_PANE = """{"id":"$BLOCKED","node_id":"01JNODE","workspace":"zeta","agent":"claude","agent_status":"blocked","updated_at":"2026-10-26T08:03:00Z"}"""

// The node's own order, which puts a fleet run first whenever its pty is the newest thing it holds.
private val FLEET_FIRST = herd(FLEET_PANE, AGENT_PANE, SHELL_PANE)
private val FLEET_FIRST_WITH_BLOCKED = herd(FLEET_PANE, AGENT_PANE, SHELL_PANE, BLOCKED_PANE)

private fun paired(): MemoryPrefs = MemoryPrefs().apply {
    set("endpoint", "http://127.0.0.1:8790")
    set("token", "kmp_stored")
}

private fun KamprStore.feed(frame: String) = accept(Wire.decode(frame) ?: error("undecodable: $frame"))

private fun opened(app: AppState): String? = (app.screen as? Screen.Pane)?.paneId

// What a previous run of the app left behind: the operator opened `pane`, then the app was closed.
private fun lastLookedAt(pane: String, frame: String): MemoryPrefs {
    val prefs = paired()
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
    val store = KamprStore().apply { feed(frame) }
    AppState(scope, store, prefs, null, NoPush()).openPane(pane)
    scope.cancel()
    return prefs
}

// The report: "the app shows me a random fleet log on open." It was whatever pane the node happened
// to list first, and a fleet run's pty usually is. The app has to come back where the operator
// left it, the way a multiplexer reattaches, and otherwise somewhere that is on their desk.
@OptIn(ExperimentalTestApi::class, InternalComposeUiApi::class)
class ReattachTest {
    private fun launch(
        prefs: MemoryPrefs,
        frame: String,
        breakpoint: MutableState<Breakpoint>,
        body: ComposeUiTest.(AppState) -> Unit,
    ) {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val store = KamprStore().apply { feed(frame) }
        val app = AppState(scope, store, prefs, null, NoPush())
        try {
            runComposeUiTest {
                setContent {
                    CompositionLocalProvider(
                        LocalTokens provides phoneTokens(),
                        LocalSafeArea provides BARS,
                        LocalCompatNavigationEventDispatcherOwner provides SystemBackWindow(),
                    ) {
                        Box(Modifier.size(1280.dp, 900.dp)) {
                            AppScaffold(
                                state = app,
                                breakpoint = breakpoint.value,
                                surfaces = BlankSurfaces,
                                mosaic = NoMosaic,
                                now = 0.0,
                                auth = AuthSurface(null, emptyList(), null, null, null, {}, {}, {}, {}),
                                connectionStatus = ConnectionStatus.Live("full"),
                                deepLink = null,
                            )
                        }
                    }
                }
                waitForIdle()
                body(app)
            }
        } finally {
            scope.cancel()
        }
    }

    private fun launch(prefs: MemoryPrefs, frame: String, breakpoint: Breakpoint, body: ComposeUiTest.(AppState) -> Unit) = launch(prefs, frame, mutableStateOf(breakpoint), body)

    @Test
    fun aFreshDesktopNeverOpensOnAFleetRun() {
        for ((frame, expected) in listOf(FLEET_FIRST to AGENT, FLEET_FIRST_WITH_BLOCKED to BLOCKED)) {
            launch(paired(), frame, Breakpoint.Desktop) { app ->
                assertEquals(expected, opened(app), "a desk with nothing remembered opened ${opened(app)}")
            }
        }
    }

    @Test
    fun theAppReopensOnThePaneTheOperatorLastOpened() {
        for (breakpoint in Breakpoint.entries) {
            launch(lastLookedAt(SHELL, FLEET_FIRST_WITH_BLOCKED), FLEET_FIRST_WITH_BLOCKED, breakpoint) { app ->
                assertEquals(SHELL, opened(app), "$breakpoint did not come back to the pane it was left on")
            }
        }
    }

    @Test
    fun aRememberedPaneThatHasGoneFallsBackAndIsNotForgotten() {
        val prefs = lastLookedAt(SHELL, FLEET_FIRST)
        val without = herd(FLEET_PANE, AGENT_PANE)
        launch(prefs, without, Breakpoint.Portrait) { app ->
            assertEquals(Screen.Herd, app.screen, "a phone with nothing to reattach to opens on the herd")
        }
        launch(prefs, without, Breakpoint.Desktop) { app ->
            assertEquals(AGENT, opened(app))
        }
        launch(prefs, FLEET_FIRST, Breakpoint.Portrait) { app ->
            assertEquals(SHELL, opened(app), "a fallback overwrote the pane the operator had chosen")
        }
    }

    // Reattaching is a launch's, not every herd frame's or every rotation's, which re-runs the
    // scaffold's launch effect.
    @Test
    fun goingBackToTheHerdStaysThereThroughHerdUpdatesAndARotation() {
        val breakpoint = mutableStateOf(Breakpoint.Portrait)
        launch(lastLookedAt(SHELL, FLEET_FIRST), FLEET_FIRST, breakpoint) { app ->
            assertEquals(SHELL, opened(app))
            app.back()
            app.store.feed(FLEET_FIRST_WITH_BLOCKED)
            breakpoint.value = Breakpoint.Landscape
            waitForIdle()
            assertEquals(Screen.Herd, app.screen)
        }
    }
}

// ADR 0015: coming back to a pane is looking at it. The node hears a watch and nothing that could
// focus, claim, resize or type into the pane.
class ReattachWritesNothingTest {
    @Test
    fun reattachingSendsOnlyAWatch() {
        val node = StubNode().apply { start() }
        val scope = CoroutineScope(SupervisorJob())
        val app = AppState(scope, KamprStore(), lastLookedAt(SHELL, FLEET_FIRST), null, NoPush())
        try {
            runBlocking {
                app.connection.connect(Endpoint("http://127.0.0.1:${node.port}"))
                until("the session") { app.store.status.value is ConnectionStatus.Live }
                node.push(FLEET_FIRST)
                until("the reattach") { opened(app) == SHELL }
                until("the watch") { node.received.any { it.contains("\"watch\"") } }
                val kinds = node.received.map { Regex(""""t":"([^"]+)"""").find(it)?.groupValues?.get(1) }.toSet()
                assertTrue(kinds.all { it in setOf("watch", "ping") }, "reattaching sent $kinds")
            }
        } finally {
            app.connection.disconnect()
            scope.cancel()
            node.stop()
        }
    }
}
