package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppManage
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.ManageLayer
import dev.kampr.shared.ui.NewSheet
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

private val NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local")
private val PEER = NodeInfo(id = "01JLOFT", name = "loft", kind = "peer")

private val SHELL = PaneInfo(
    id = "01JNODE/w3:p2",
    nodeId = "01JNODE",
    workspaceId = "01JNODE/w3",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
)

private val BUSY = PaneInfo(
    id = "01JNODE/w4:p1",
    nodeId = "01JNODE",
    workspaceId = "01JNODE/w4",
    workspace = "herdr",
    agent = "codex",
)

private val ELSEWHERE = PaneInfo(id = "01JLOFT/w1:p1", nodeId = "01JLOFT", workspace = "loft-work")

private val CAPS = ServerMsg.NodeCaps(
    node = "01JNODE",
    agentKinds = listOf("claude", "codex"),
    sessions = emptyList(),
)

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), content = content)
}

// Reported from a phone: "new session dialogue - if i select claude on start an agent the start
// button is disabled?" It was, permanently, and only when the sheet had been opened from the
// herd's own + — which has no pane, and `agent.start` takes one. The card carried a line saying
// so; it was in the section above the button that was refusing, and the operator read a broken
// button rather than an explanation.
@OptIn(ExperimentalTestApi::class)
class NewSheetAgentPaneTest {
    @Test
    fun anAgentStartedFromTheHerdChoosesThePaneItRunsIn() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent {
            Themed {
                Box(Modifier.size(420.dp, 900.dp)) {
                    sheet(sent, panes = listOf(SHELL, BUSY, ELSEWHERE))
                }
            }
        }

        onNodeWithContentDescription("Start a claude agent").performClick()
        waitForIdle()
        onNodeWithContentDescription("Start claude").performClick()
        waitForIdle()
        assertTrue(sent.isEmpty(), "the button ran with no pane to run in")

        // A pane on another machine is not this node's to start an agent in, and its harnesses
        // are its own — `caps.agentKinds` here is the connected node's answer.
        assertEquals(
            0,
            onAllNodesWithContentDescription("Start it in loft-work · bash").fetchSemanticsNodes().size,
            "a peer's pane was offered as somewhere to start this node's agent",
        )

        onNodeWithContentDescription("Start it in kampr · bash").performClick()
        waitForIdle()
        onNodeWithContentDescription("Start claude").performClick()
        waitForIdle()

        assertEquals(
            listOf(ManageOp.AgentStart(SHELL.id, "claude", null, emptyList()) as ManageOp),
            sent,
        )
    }

    // The other half of never leaving a dead button unexplained: a machine with no panes at all
    // has nothing to pick, so the reason has to be said beside the button rather than implied by
    // an empty row of chips.
    @Test
    fun aMachineWithNoPanesSaysWhyTheAgentCannotStart() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent {
            Themed { Box(Modifier.size(420.dp, 900.dp)) { sheet(sent, panes = listOf(ELSEWHERE)) } }
        }

        onNodeWithContentDescription("Start a claude agent").performClick()
        waitForIdle()

        onNodeWithContentDescription("Start claude").performClick()
        waitForIdle()
        onNodeWithContentDescription(
            "There is no pane on comingclean to start claude in — make a workspace first.",
        ).assertExists()
        assertTrue(sent.isEmpty())
    }

    // A pane the sheet was opened *from* is the target, and picking a kind is the whole of it —
    // the chooser must not appear and take a second tap that was never needed.
    @Test
    fun aSheetOpenedFromAPaneStillStartsTheAgentInThatPane() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent {
            Themed {
                Box(Modifier.size(420.dp, 900.dp)) {
                    sheet(sent, pane = BUSY, panes = listOf(SHELL, BUSY))
                }
            }
        }

        onNodeWithContentDescription("Start a claude agent").performClick()
        waitForIdle()
        assertEquals(
            0,
            onAllNodesWithContentDescription("Start it in kampr · bash").fetchSemanticsNodes().size,
            "the pane is already decided, so there is nothing to choose",
        )
        onNodeWithContentDescription("Start claude").performClick()
        waitForIdle()

        assertEquals(BUSY.id, (sent.single() as ManageOp.AgentStart).at)
    }

    // The sheet cannot offer panes it was never given. Driven through `ManageLayer` because that
    // is the only place the herd's panes reach it.
    @Test
    fun theHerdsPlusButtonHandsTheSheetEveryPaneOnTheMachine() = runComposeUiTest {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val store = KamprStore()
        store.take(MANAGING)
        store.take(HERD_FRAME)
        store.take(CAPS_FRAME)
        val state = AppState(scope, store, MemoryPrefs(), null)
        try {
            AppManage(state).openNew(null)
            setContent {
                Themed {
                    Box(Modifier.size(420.dp, 900.dp)) {
                        ManageLayer(state, Herd(listOf(NODE), listOf(SHELL), known = true), Breakpoint.Portrait)
                    }
                }
            }
            waitForIdle()
            onNodeWithContentDescription("Start a claude agent").performClick()
            waitForIdle()
            onNodeWithContentDescription("Start it in kampr · bash").assertExists()
        } finally {
            scope.cancel()
        }
    }
}

private const val MANAGING =
    """{"t":"hello","protocol":1,"node_id":"01JNODE","node_name":"comingclean","build":"test",""" +
        """"role":"full","caps":{"manage":true},"security":{"tier":0,"passkeys":false}}"""

private const val HERD_FRAME =
    """{"t":"herd","nodes":[{"id":"01JNODE","name":"comingclean","kind":"local"}],""" +
        """"panes":[{"id":"01JNODE/w3:p2","node_id":"01JNODE","workspace":"kampr"}]}"""

private const val CAPS_FRAME =
    """{"t":"caps","node":"01JNODE","agent_kinds":["claude","codex"],"sessions":[]}"""

private fun KamprStore.take(frame: String) = accept(Wire.decode(frame) ?: error("undecodable: $frame"))

@Composable
private fun sheet(sent: MutableList<ManageOp>, pane: PaneInfo? = null, panes: List<PaneInfo>) {
    NewSheet(
        breakpoint = Breakpoint.Portrait,
        node = NODE,
        pane = pane,
        nodes = listOf(NODE, PEER),
        caps = CAPS,
        outcome = null,
        onManage = { sent += it },
        onNode = {},
        onNodePicker = {},
        onDismiss = {},
        panes = panes,
    )
}
