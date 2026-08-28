package dev.kampr.shared

import androidx.compose.foundation.layout.Box
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
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.AppScaffold
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.AuthSurface
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.NoMosaic
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertTrue

private const val HERE = "01JHERE/w1:p1"
private const val NOW = 1_787_000_000_000.0

private const val HERD = """
    {"t":"herd",
     "nodes":[{"id":"01JHERE","name":"comingclean","kind":"local","online":true},
              {"id":"01JTHERE","name":"workbox","kind":"peer","online":true}],
     "panes":[
       {"id":"01JHERE/w1:p1","node_id":"01JHERE","workspace":"kampr","tab":"1",
        "cwd":"/home/dbrain/dev/kampr","agent":"claude","agent_status":"idle",
        "cols":94,"rows":40,"has_conversation":true},
       {"id":"01JTHERE/w2:p4","node_id":"01JTHERE","workspace":"herdr","tab":"1",
        "cwd":"/home/dbrain/dev/herdr","agent":"codex","agent_status":"idle",
        "cols":94,"rows":40,"has_conversation":true}]}
"""

private const val ELSEWHERE_OFFLINE =
    """{"t":"error","code":"node_offline","message":"workbox is offline","node":"01JTHERE"}"""

private const val HERE_OFFLINE =
    """{"t":"error","code":"node_offline","message":"comingclean is offline","node":"01JHERE"}"""

private const val REVOKED =
    """{"t":"error","code":"revoked","message":"this device was revoked"}"""

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

private class BlankPaneSurfaces : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) = Box(modifier.size(1.dp))
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
private fun ComposeUiTest.phone(state: AppState) {
    setContent {
        CompositionLocalProvider(LocalTokens provides tokens()) {
            Box(Modifier.size(390.dp, 844.dp)) {
                AppScaffold(
                    state = state,
                    breakpoint = Breakpoint.Portrait,
                    surfaces = BlankPaneSurfaces(),
                    mosaic = NoMosaic,
                    now = NOW,
                    auth = AuthSurface(null, emptyList(), null, null, null, {}, {}, {}, {}),
                    connectionStatus = ConnectionStatus.Live("full"),
                    deepLink = null,
                )
            }
        }
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.strips(words: String): Int =
    onAllNodesWithContentDescription(words, substring = true).fetchSemanticsNodes().size

// The operator's rule, verbatim: "i just don't want to hear about something not relevant to the
// node im on loudly on mobile, if the thing im using disconnects thats a different thing."
@OptIn(ExperimentalTestApi::class)
class QuietNodeTest {
    @Test
    fun aNodeGoingOfflineElsewhereDoesNotInterruptThePaneInHand() = runComposeUiTest {
        val (state, scope) = app()
        try {
            state.openPane(HERE)
            phone(state)
            state.store.accept(Wire.decode(ELSEWHERE_OFFLINE) ?: error("undecodable"))
            waitForIdle()
            assertTrue(
                strips("workbox is offline") == 0,
                "a node the operator is not on took the screen over the pane they are on",
            )
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun theNodeThePaneInHandIsOnIsStillSaidOutLoud() = runComposeUiTest {
        val (state, scope) = app()
        try {
            state.openPane(HERE)
            phone(state)
            state.store.accept(Wire.decode(HERE_OFFLINE) ?: error("undecodable"))
            waitForIdle()
            onNodeWithContentDescription("comingclean is offline", substring = true).assertExists()
        } finally {
            scope.cancel()
        }
    }

    // Auth, a revocation, the socket itself: nothing names a pane or a node, and there is nowhere
    // quieter for them to go.
    @Test
    fun aRefusalAboutNeitherAPaneNorANodeIsStillLoudWhereverTheOperatorIs() = runComposeUiTest {
        val (state, scope) = app()
        try {
            state.openPane(HERE)
            phone(state)
            state.store.accept(Wire.decode(REVOKED) ?: error("undecodable"))
            waitForIdle()
            onNodeWithContentDescription("this device was revoked", substring = true).assertExists()
        } finally {
            scope.cancel()
        }
    }

    // The herd screen is where an offline node belongs, and it says so there whether or not the
    // strip ever appeared.
    @Test
    fun aNodeGoingOfflineIsNotSaidOutLoudOnTheHerdEither() = runComposeUiTest {
        val (state, scope) = app()
        try {
            phone(state)
            state.store.accept(Wire.decode(ELSEWHERE_OFFLINE) ?: error("undecodable"))
            waitForIdle()
            assertTrue(strips("workbox is offline") == 0, "a herd list interrupted itself")
        } finally {
            scope.cancel()
        }
    }
}
