package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.NewSheet
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// A host nobody has visited for a month: kampr answers, herdr does not. The node is offline and
// has no panes, and a manage op sent to it starts the herdr it needs (probes #324, #325) — so the
// sheet has to let the operator send one. Before this, every machine that was offline was
// unpickable, and the one whose herdr was merely stopped was the common case of the three.
private val COLD = NodeInfo(id = "01JCOLD", name = "coldbox", kind = "local", online = false, detail = "herdr socket /c/herdr/herdr.sock: No such file or directory")
private val GONE = NodeInfo(id = "01JGONE", name = "gonebox", kind = "peer", online = false)

@OptIn(ExperimentalTestApi::class)
class NewSheetColdHostTest {
    @Test
    fun aWorkspaceCanBeMadeOnALocalNodeWhoseHerdrIsNotRunning() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent {
            Themed {
                Box(Modifier.size(420.dp, 900.dp)) {
                    NewSheet(
                        breakpoint = Breakpoint.Portrait,
                        node = COLD,
                        pane = null,
                        nodes = listOf(COLD),
                        caps = ServerMsg.NodeCaps(node = COLD.id, agentKinds = emptyList(), sessions = emptyList()),
                        outcome = null,
                        onManage = { sent += it },
                        onNode = {},
                        onNodePicker = {},
                        onDismiss = {},
                        onRefreshCaps = {},
                    )
                }
            }
        }

        onNodeWithContentDescription("Create workspace").performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        assertEquals(listOf(ManageOp.WorkspaceCreate(COLD.id) as ManageOp), sent)
    }

    @Test
    fun theMachineListOffersAStoppedHerdrAndStillRefusesAnUnreachablePeer() = runComposeUiTest {
        var aimed: String? = null
        setContent {
            Themed {
                Box(Modifier.size(420.dp, 900.dp)) {
                    NewSheet(
                        breakpoint = Breakpoint.Portrait,
                        node = COLD,
                        pane = null,
                        nodes = listOf(COLD, GONE),
                        caps = ServerMsg.NodeCaps(node = COLD.id, agentKinds = emptyList(), sessions = emptyList()),
                        outcome = null,
                        onManage = {},
                        onNode = { aimed = it },
                        onNodePicker = {},
                        onDismiss = {},
                        onRefreshCaps = {},
                    )
                }
            }
        }

        onNodeWithContentDescription("Change machine, currently coldbox").performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()

        // The stopped herdr says what is actually true of it, and what making something will do —
        // `detail` there is a socket path and an errno, which answers a question nobody asked.
        onNodeWithText("herdr is not running here — making something starts it", useUnmergedTree = true)
            .assertExists()
        onNodeWithContentDescription("Create on coldbox").performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        assertEquals(COLD.id, aimed)

        // A peer that is offline may be a stopped herdr or a dead link, and nothing here can tell.
        assertTrue(
            runCatching {
                onNodeWithContentDescription("Create on gonebox").performSemanticsAction(SemanticsActions.OnClick)
            }.isFailure,
            "an unreachable peer was offered as somewhere to create",
        )
    }
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), content = content)
}
