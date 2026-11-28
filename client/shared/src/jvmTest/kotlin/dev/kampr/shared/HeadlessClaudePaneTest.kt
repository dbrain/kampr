package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.model.groups
import dev.kampr.shared.model.sections
import dev.kampr.shared.model.serves
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.MenuAnchor
import dev.kampr.shared.ui.PaneMenu
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.named
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val HEADLESS_ID = "01JNODE/c:ca931d887e64f96cd"
private const val TERMINAL_SURFACE = "Terminal surface probe"
private const val CONVERSATION_SURFACE = "Conversation surface probe"
private const val NO_PICTURE = "This pane has no picture"
private const val REFUSED = "01JNODE/c:ca931d887e64f96cd is not a pane, tab or workspace on this node"

// What the node sends for a Claude it started with no terminal, beside a herdr pane in a named
// workspace and a native session in none.
private const val HERD = """
    {"t":"herd",
     "nodes":[{"id":"01JNODE","name":"comingclean","kind":"local","online":true,"native":true,"claude":true}],
     "panes":[
       {"id":"01JNODE/c:ca931d887e64f96cd","node_id":"01JNODE","cwd":"/home/dbrain/dev/stallabout",
        "agent":"claude","agent_status":"idle","rows":0,"has_conversation":false,"converses":true},
       {"id":"01JNODE/w1:p1","node_id":"01JNODE","workspace_id":"01JNODE/w1","workspace":"pissant",
        "agent_status":"idle","rows":24},
       {"id":"01JNODE/n:s1","node_id":"01JNODE","provider":"native","session_id":"s1","agent_status":"idle","rows":24}]}
"""

private fun herd(): ServerMsg.Herd = Wire.decode(HERD) as ServerMsg.Herd

private fun headless(): PaneInfo = herd().panes.first { it.id == HEADLESS_ID }

private class HeadlessProbe : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier.named(TERMINAL_SURFACE))

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier.named(CONVERSATION_SURFACE))

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) = Box(modifier.size(40.dp))
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.count(label: String): Int = onAllNodesWithContentDescription(label, substring = true, useUnmergedTree = true).fetchSemanticsNodes().size

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.texts(): List<String> = onAllNodesWithText("", substring = true)
    .fetchSemanticsNodes()
    .mapNotNull { node -> node.config.getOrElseNullable(SemanticsProperties.Text) { null }?.joinToString("") { it.text } }

// docs/21: a Claude with no terminal is a conversation and nothing else. It used to open on a
// terminal that could not be watched, list under a workspace "c" read off its id, and close
// through herdr's verb, which the node refused.
@OptIn(ExperimentalTestApi::class)
class HeadlessClaudePaneTest {
    @Test
    fun itOpensOnItsConversationWhereverItIsOpened() {
        for (compact in listOf(true, false)) {
            val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
            try {
                val store = KamprStore().apply { accept(herd()) }
                store.accept(Wire.decode("""{"t":"prefs","panes":{"$HEADLESS_ID":{"view":"terminal"}}}""")!!)
                val app = AppState(scope, store, MemoryPrefs(), null).apply { go(Screen.Herd) }
                app.compact = compact
                app.openPane(HEADLESS_ID)
                assertEquals(PaneView.Conversation, (app.screen as Screen.Pane).view, "compact=$compact")
            } finally {
                scope.cancel()
            }
        }
    }

    // New → Start conversation: the ack names the pane before the herd that lists it arrives.
    @Test
    fun startingOneOpensItsConversationOnceTheHerdListsIt() {
        for (compact in listOf(true, false)) {
            val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
            try {
                val store = KamprStore()
                val without = herd().copy(panes = herd().panes.filterNot { it.id == HEADLESS_ID })
                store.accept(without)
                val app = AppState(scope, store, MemoryPrefs(), null).apply { go(Screen.Herd) }
                app.compact = compact
                app.opening(HEADLESS_ID)
                assertEquals(Screen.Herd, app.screen, "opened before it was listed")
                store.accept(herd())
                assertEquals(Screen.Pane(HEADLESS_ID, PaneView.Conversation, Screen.Herd), app.screen, "compact=$compact")
            } finally {
                scope.cancel()
            }
        }
    }

    @Test
    fun itIsNeverATerminalThatFailedToPaint() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(1280.dp, 860.dp)) {
                    PaneScreenDesktop(
                        PaneState(HEADLESS_ID, StyleTable()).apply { refusal = REFUSED },
                        headless().copy(detail = REFUSED),
                        PaneView.Terminal,
                        HeadlessProbe(),
                        readOnly = false,
                        onView = {},
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        waitForIdle()
        onNodeWithContentDescription(CONVERSATION_SURFACE).assertIsDisplayed()
        assertEquals(0, count(TERMINAL_SURFACE), "given a terminal")
        assertEquals(0, count(NO_PICTURE), "said to have failed to paint")
        assertEquals(0, onAllNodesWithText(NO_PICTURE).fetchSemanticsNodes().size, "stream notice")
    }

    @Test
    fun itIsListedInTheBackgroundAndNotUnderAWorkspaceReadOffItsId() {
        val m = herd()
        val group = Herd(m.nodes, m.panes).groups().single()
        assertEquals(listOf(HEADLESS_ID), group.background.map { it.id })
        assertTrue(group.terminals.none { it.id == HEADLESS_ID })
        assertEquals(listOf(null), group.sections.map { it.heading }.filter { it != "pissant" })
        assertTrue(group.sections.flatMap { it.panes }.none { it.id == HEADLESS_ID })
    }

    @Test
    fun theListMenuEndsTheConversation() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(900.dp, 600.dp)) {
                    PaneMenu(Breakpoint.Desktop, headless(), MenuAnchor(120.dp, 120.dp), null, { sent += it }, {}, {}, desk = true)
                }
            }
        }
        waitForIdle()
        assertEquals(listOf("Open", "End…"), texts())
        onNodeWithText("End…").performClick()
        waitForIdle()
        onNodeWithText("End session").performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.ClaudeEnd(HEADLESS_ID)), sent)
    }

    @Test
    fun itNeedsOnlyItsNodeAndNotItsHerdr() {
        val cold = NodeInfo("01JNODE", "comingclean", "peer", online = false, reachable = true, native = true)
        assertTrue(cold.serves(headless()))
    }
}
