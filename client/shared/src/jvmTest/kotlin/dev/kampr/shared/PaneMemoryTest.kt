package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
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
import dev.kampr.shared.ui.PaneScreenDesktop
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

private const val CLAUDE = "01JNODE/w3:p1"
private const val CODEX = "01JNODE/w4:p1"

// Two agent panes with no ring at all, which is what a live harness actually reports: Claude Code
// clears the scrollback when it takes the screen (#231), so `scrollback_rows` is 0 on exactly the
// panes this product exists for. The live grid is streamed regardless — the ring only decides
// whether there is history above it.
private const val HERD = """
    {"t":"herd",
     "nodes":[{"id":"01JNODE","name":"comingclean","kind":"local","online":true,
               "herdr_version":"0.8.2","build":"0.1.0+abc1234"}],
     "panes":[
       {"id":"01JNODE/w3:p1","node_id":"01JNODE","workspace":"kampr","tab":"1",
        "cwd":"/home/dbrain/dev/kampr","agent":"claude","agent_status":"idle",
        "cols":94,"rows":40,"scrollback_rows":0,"has_conversation":true,
        "updated_at":"2026-08-21T13:44:02Z"},
       {"id":"01JNODE/w4:p1","node_id":"01JNODE","workspace":"herdr","tab":"1",
        "cwd":"/home/dbrain/dev/herdr","agent":"codex","agent_status":"idle",
        "cols":94,"rows":40,"scrollback_rows":0,"has_conversation":true,
        "updated_at":"2026-08-21T13:44:02Z"}]}
"""

// The greeting's third frame when this device has never chosen anything, which the node sends
// whether or not it holds a row.
private const val NO_PREFS = """{"t":"prefs","panes":{}}"""

private fun remembering(vararg views: Pair<String, PaneView>): String =
    views.joinToString(",", """{"t":"prefs","panes":{""", "}}") { (pane, view) ->
        """"$pane":{"view":"${view.key}"}"""
    }

private fun KamprStore.feed(frame: String) = accept(Wire.decode(frame) ?: error("undecodable: $frame"))

private fun paired(): MemoryPrefs = MemoryPrefs().apply {
    set("endpoint", "http://127.0.0.1:8790")
    set("token", "kmp_stored")
}

private fun state(vararg greeting: String): Pair<AppState, CoroutineScope> {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
    val store = KamprStore()
    for (frame in greeting) store.feed(frame)
    return AppState(scope, store, paired(), null) to scope
}

private fun opened(app: AppState): Pair<String, PaneView>? =
    (app.screen as? Screen.Pane)?.let { it.paneId to it.view }

private fun tokens(scale: TypeScale) = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, scale))
}

private class ProbeSurfaces : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) = Box(modifier.size(1.dp))
}

// What the operator picked for a pane, against what the app decides for them. The node stores the
// choice per pane and pushes it back on every connection, and the desktop was overriding it on
// every open with a hard-coded Split — so "click Terminal" survived exactly until the next open.
class PaneMemoryTest {
    @Test
    fun aPaneWithNothingRememberedOpensInTheTerminalEvenWhenItTalks() {
        val (app, scope) = state(HERD, NO_PREFS)
        try {
            app.openPane(CLAUDE)
            assertEquals(
                CLAUDE to PaneView.Terminal,
                opened(app),
                "a pane this device has said nothing about opens on the terminal",
            )
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun aPaneOpenedBeforeItsPrefsArriveMovesToTheViewThisDeviceRemembers() {
        // `herd` is the second frame and `prefs` the third: this is the order on the wire, and the
        // desktop opens a pane the moment the herd lands.
        val (app, scope) = state(HERD)
        try {
            app.openPane(CLAUDE)
            assertEquals(CLAUDE to PaneView.Terminal, opened(app), "nothing is known yet")

            app.store.feed(remembering(CLAUDE to PaneView.Conversation))
            assertEquals(
                CLAUDE to PaneView.Conversation,
                opened(app),
                "the memory arrived a frame late and the guess was never revisited",
            )
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun aPrefsFrameThatLandsAfterTheOperatorHasChosenDoesNotOverruleThem() {
        val (app, scope) = state(HERD)
        try {
            app.openPane(CLAUDE)
            app.setPaneView(PaneView.Split)
            app.store.feed(remembering(CLAUDE to PaneView.Conversation))
            assertEquals(
                CLAUDE to PaneView.Split,
                opened(app),
                "a frame in flight since the greeting undid a choice made after it",
            )
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun aViewChosenOnOnePaneIsNotSpentOnTheNextOneOpened() {
        val (app, scope) = state(HERD, remembering(CLAUDE to PaneView.Conversation))
        try {
            app.openPane(CLAUDE)
            assertEquals(CLAUDE to PaneView.Conversation, opened(app))
            app.openPane(CODEX)
            assertEquals(CODEX to PaneView.Terminal, opened(app), "the memory is per pane")
        } finally {
            scope.cancel()
        }
    }
}

// The desktop, which is where the defect was reported: every pane opened in Split, on the sidebar
// and on the automatic open behind the herd, whatever the operator had chosen.
@OptIn(ExperimentalTestApi::class)
class DesktopPaneMemoryTest {
    private fun ComposeUiTest.show(app: AppState) {
        setContent {
            CompositionLocalProvider(LocalTokens provides tokens(TypeScale.Desk)) {
                Box(Modifier.size(1280.dp, 860.dp)) {
                    AppScaffold(
                        state = app,
                        breakpoint = Breakpoint.Desktop,
                        surfaces = ProbeSurfaces(),
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

    @Test
    fun theDesktopOpensAPaneOnTheViewThisDeviceChoseAndNotSplit() = runComposeUiTest {
        val (app, scope) = state(HERD, remembering(CLAUDE to PaneView.Conversation))
        try {
            show(app)
            assertEquals(
                CLAUDE to PaneView.Conversation,
                opened(app),
                "the desktop opened the first pane in a view nobody asked for",
            )
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun aDesktopPaneWithNothingRememberedOpensInTheTerminal() = runComposeUiTest {
        val (app, scope) = state(HERD, NO_PREFS)
        try {
            show(app)
            assertEquals(CLAUDE to PaneView.Terminal, opened(app))
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun theSidebarOpensAPaneOnTheViewThisDeviceChoseAndNotSplit() = runComposeUiTest {
        val (app, scope) = state(HERD, remembering(CODEX to PaneView.Terminal))
        try {
            show(app)
            onNodeWithContentDescription("Open herdr · codex", substring = true).performClick()
            waitForIdle()
            assertEquals(
                CODEX to PaneView.Terminal,
                opened(app),
                "the sidebar overrode the operator's choice with Split",
            )
        } finally {
            scope.cancel()
        }
    }

    // The one that fails on a cold connection and passes on a warm one, which is the difference
    // between this being fixed and it looking fixed: `prefs` is the frame after `herd`, and the
    // desktop opens its pane on `herd`.
    @Test
    fun aDesktopThatOpensAPaneBeforeItsPrefsArriveStillLandsOnTheRememberedView() = runComposeUiTest {
        val (app, scope) = state(HERD)
        try {
            show(app)
            assertEquals(CLAUDE to PaneView.Terminal, opened(app), "nothing is known yet")

            app.store.feed(remembering(CLAUDE to PaneView.Conversation))
            waitForIdle()
            assertEquals(
                CLAUDE to PaneView.Conversation,
                opened(app),
                "the pane was opened before its prefs landed and never caught up",
            )
        } finally {
            scope.cancel()
        }
    }
}

// Split stays reachable — it is the right answer on a wide monitor for some people — but it is no
// longer what a pane opens as, so it is no longer what the switch leads with.
@OptIn(ExperimentalTestApi::class)
class PaneSwitchOrderTest {
    private val info = PaneInfo(
        id = CLAUDE,
        nodeId = "01JNODE",
        workspace = "kampr",
        agent = "claude",
        agentStatus = "idle",
        cols = 94,
        rows = 40,
        hasConversation = true,
    )

    @Test
    fun theWideSwitchOffersTerminalFirstAndSplitLast() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides tokens(TypeScale.Desk)) {
                Box(Modifier.size(1280.dp, 860.dp)) {
                    PaneScreenDesktop(
                        pane = PaneState(CLAUDE, StyleTable()),
                        info = info,
                        view = PaneView.Terminal,
                        surfaces = ProbeSurfaces(),
                        readOnly = false,
                        onView = {},
                        onAnswer = {},
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        waitForIdle()
        val terminal = leftEdgeOf("Terminal view")
        val conversation = leftEdgeOf("Conversation view")
        val split = leftEdgeOf("Split view")
        assertTrue(terminal < conversation, "Terminal is not first: $terminal, $conversation, $split")
        assertTrue(conversation < split, "Split is not last: $terminal, $conversation, $split")
    }

    private fun ComposeUiTest.leftEdgeOf(label: String): Float =
        onNodeWithContentDescription(label).fetchSemanticsNode().boundsInRoot.left
}

private const val NOW = 1_787_000_000_000.0
