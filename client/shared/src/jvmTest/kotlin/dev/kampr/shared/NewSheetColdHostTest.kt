package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithText
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

// The report the peer half came from: a server was rebooted, its kampr came back as a service and
// its herdr did not, and the sheet refused it. `online` is the machine's *herdr*; `reachable` is
// its *kampr*, and the mesh sets it from the link — "the link is up, so the peer's node process is
// answering, whatever its own herdr is doing". Asking `kind == "local"` was asking a different
// question that happens to share an answer on exactly one machine in the herd.
private val SHED = NodeInfo(
    id = "01JSHED",
    name = "shed",
    kind = "peer",
    online = false,
    detail = "herdr socket /run/user/1000/herdr/default.sock: No such file or directory",
    reachable = true,
)

// A peer whose link is up and whose node says it cannot serve. Believed, and refused.
private val REFUSING = NodeInfo(id = "01JREF", name = "refusing", kind = "peer", online = false, reachable = false)

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

        // A peer that is offline and says nothing about its node is one an older build served, and
        // `online` is then the only evidence there is.
        assertTrue(
            runCatching {
                onNodeWithContentDescription("Create on gonebox").performSemanticsAction(SemanticsActions.OnClick)
            }.isFailure,
            "an unreachable peer was offered as somewhere to create",
        )
    }

    @Test
    fun aRebootedPeerWhoseNodeIsUpIsSomewhereToCreate() = runComposeUiTest {
        var aimed: String? = null
        setContent {
            Themed {
                Box(Modifier.size(420.dp, 900.dp)) {
                    NewSheet(
                        breakpoint = Breakpoint.Portrait,
                        node = COLD,
                        pane = null,
                        nodes = listOf(COLD, SHED, REFUSING),
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

        // Not the socket path and the errno, which is what it said before: the same sentence the
        // local cold host gets, because it is the same situation and the same remedy — so both of
        // them say it and the machine with no node behind it does not.
        assertEquals(
            2,
            onAllNodesWithText("herdr is not running here — making something starts it", useUnmergedTree = true)
                .fetchSemanticsNodes().size,
            "a stopped herdr reads differently depending on whose machine it is on",
        )
        onNodeWithContentDescription("Create on shed").performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        assertEquals(SHED.id, aimed)
    }

    // The other half of the same mistake. Widening this to "offline is fine" would offer a machine
    // whose own node has said it cannot serve, and the op would fail after the operator had chosen.
    @Test
    fun aPeerWhoseNodeSaysItCannotServeIsStillRefused() = runComposeUiTest {
        setContent {
            Themed {
                Box(Modifier.size(420.dp, 900.dp)) {
                    NewSheet(
                        breakpoint = Breakpoint.Portrait,
                        node = COLD,
                        pane = null,
                        nodes = listOf(COLD, SHED, REFUSING),
                        caps = ServerMsg.NodeCaps(node = COLD.id, agentKinds = emptyList(), sessions = emptyList()),
                        outcome = null,
                        onManage = {},
                        onNode = {},
                        onNodePicker = {},
                        onDismiss = {},
                        onRefreshCaps = {},
                    )
                }
            }
        }

        onNodeWithContentDescription("Change machine, currently coldbox").performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        assertTrue(
            runCatching {
                onNodeWithContentDescription("Create on refusing").performSemanticsAction(SemanticsActions.OnClick)
            }.isFailure,
            "a peer whose node reported itself unreachable was offered as somewhere to create",
        )
    }
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), content = content)
}
