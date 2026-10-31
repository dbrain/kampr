package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.NewSheet
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PROVIDER_HERDR
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals

private val NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local", online = true, reachable = true, native = true, herdr = true)
private val CAPS = ServerMsg.NodeCaps("01JNODE", agentKinds = listOf("claude", "amp"), sessions = emptyList(), nativeAgents = listOf("claude", "codex"))
private val IN_WORKSPACE = PaneInfo(
    id = "01JNODE/n:01ja",
    nodeId = "01JNODE",
    provider = "native",
    workspaceId = "01JNODE/kw:api",
    workspace = "api",
    cwd = "/srv/api",
)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.sheet(pane: PaneInfo? = null, sent: MutableList<ManageOp>) = setContent {
    CompositionLocalProvider(LocalTokens provides phoneTokens()) {
        Box(Modifier.size(420.dp, 2400.dp)) {
            NewSheet(
                breakpoint = Breakpoint.Portrait,
                node = NODE,
                pane = pane,
                nodes = listOf(NODE),
                caps = CAPS,
                outcome = null,
                onManage = { sent += it },
                onNode = {},
                onNodePicker = {},
                onDismiss = {},
            )
        }
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.press(description: String) {
    onNodeWithContentDescription(description, substring = true).performSemanticsAction(SemanticsActions.OnClick)
    waitForIdle()
}

// Workspaces, tabs, agents and worktrees are made as Kampr sessions unless the operator picked
// herdr's by name, and then the op says so: a node routes an op with no provider to Kampr.
@OptIn(ExperimentalTestApi::class)
class NewSheetKamprStructureTest {
    @Test
    fun anAgentChipStartsTheHarnessAsAKamprSession() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        sheet(sent = sent)
        waitForIdle()
        press("Start codex as a Kampr session")
        onNodeWithContentDescription("Start codex").performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.AgentStart(kind = "codex", node = NODE.id)), sent)
    }

    @Test
    fun anAgentStartedFromAKamprPaneOpensBesideIt() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        sheet(IN_WORKSPACE, sent)
        waitForIdle()
        press("Start claude as a Kampr session")
        onNodeWithContentDescription("Start claude").performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.AgentStart(kind = "claude", at = IN_WORKSPACE.id)), sent)
    }

    @Test
    fun aKamprWorkspaceIsMadeWithNoProvider() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        sheet(sent = sent)
        waitForIdle()
        press("Kampr workspace,")
        onNodeWithContentDescription("label").performTextInput("api")
        onNodeWithContentDescription("Create workspace").performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.WorkspaceCreate(NODE.id, label = "api")), sent)
    }

    @Test
    fun aKamprTabGoesInTheWorkspaceOfTheKamprPaneItWasAskedFrom() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        sheet(IN_WORKSPACE, sent)
        waitForIdle()
        press("Kampr tab,")
        onNodeWithContentDescription("Create tab").performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.TabCreate("01JNODE/kw:api", cwd = "/srv/api")), sent)
    }

    @Test
    fun aKamprWorktreeIsMadeWithNoProvider() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        sheet(IN_WORKSPACE, sent)
        waitForIdle()
        press("Kampr worktree,")
        onNodeWithContentDescription("branch").performTextInput("feat/x")
        onNodeWithContentDescription("Create worktree").performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.WorktreeCreate(NODE.id, "feat/x", cwd = "/srv/api")), sent)
    }

    @Test
    fun herdrsOwnWorkspaceSaysItIsHerdrs() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        sheet(sent = sent)
        waitForIdle()
        press("Workspace, a herdr workspace")
        onNodeWithContentDescription("Create workspace").performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.WorkspaceCreate(NODE.id, provider = PROVIDER_HERDR)), sent)
    }
}
