package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
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
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.SessionInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private val HOST = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local")

private fun capsOf(vararg sessions: SessionInfo) = ServerMsg.NodeCaps(
    node = HOST.id,
    agentKinds = listOf("claude"),
    sessions = sessions.toList(),
)

private class Sheet {
    val sent = mutableListOf<ManageOp>()
    var dismissed = false
    var refreshes = 0
    val outcome = mutableStateOf<ServerMsg.Managed?>(null)
    val caps = mutableStateOf(capsOf())

    fun acknowledge(op: String, ok: Boolean = true) {
        outcome.value = ServerMsg.Managed(op = op, ok = ok, id = null)
    }
}

// Reported from a phone, as two defects that were one: "creating a new session - session doesn't
// open when done" and "closing a session - session doesn't close when done". The ops themselves
// worked. The sheet closed on the ack — and a named session is the one thing this sheet makes
// that is *not* revealed by closing it: it is its own herdr server, it joins the herd as a node
// with no panes, and the only place it is ever drawn is this sheet's own list. So the operator
// was taken away from the single surface that was about to show the result, and the list itself
// would not have moved anyway, because it is `caps.sessions` and the node caches that answer.
@OptIn(ExperimentalTestApi::class)
class NewSheetSessionTest {
    @Test
    fun creatingANamedSessionStaysOnTheListItJustChangedAndReAsksForIt() = runComposeUiTest {
        val sheet = Sheet()
        setContent { Themed { Box(Modifier.size(420.dp, 900.dp)) { sheet.render() } } }

        onNodeWithContentDescription("Named session, its own server")
            .performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        onNodeWithContentDescription("name").performTextInput("agents")
        waitForIdle()
        onNodeWithContentDescription("Start session").performClick()
        waitForIdle()
        assertEquals(listOf(ManageOp.SessionCreate(HOST.id, "agents") as ManageOp), sheet.sent)

        sheet.acknowledge("session.create")
        waitForIdle()

        assertFalse(sheet.dismissed, "the sheet closed on the one surface that shows a session")
        assertEquals(1, sheet.refreshes, "nothing re-asked the node what sessions exist")

        // And the answer, when it arrives, lands in the list the operator is still looking at.
        sheet.caps.value = capsOf(SessionInfo("agents", running = true))
        waitForIdle()
        onNodeWithContentDescription("agents, running").assertExists()
    }

    @Test
    fun stoppingANamedSessionStaysOnTheListAndReAsksForIt() = runComposeUiTest {
        val sheet = Sheet()
        sheet.caps.value = capsOf(SessionInfo("agents", running = true))
        setContent { Themed { Box(Modifier.size(420.dp, 900.dp)) { sheet.render() } } }

        onNodeWithContentDescription("Named session, its own server · agents")
            .performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        onNodeWithContentDescription("Stop the agents session")
            .performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        assertEquals(listOf(ManageOp.SessionStop(HOST.id, "agents") as ManageOp), sheet.sent)

        sheet.acknowledge("session.stop")
        waitForIdle()

        assertFalse(sheet.dismissed, "the sheet closed before it could show the session had gone")
        assertEquals(1, sheet.refreshes)

        // A stopped session stays listed and stops being running (#242), so this is the change
        // the operator was waiting for and it has to be on screen.
        sheet.caps.value = capsOf(SessionInfo("agents", running = false))
        waitForIdle()
        onNodeWithContentDescription("agents, stopped").assertExists()
    }

    // `served` is the set of session names this node actually reaches, and the wire protocol is
    // explicit that a client must not offer to open a pane on a session that will never appear in
    // the herd. The node had computed it for months and no client had ever read it.
    @Test
    fun aSessionThisNodeDoesNotServeSaysSoRatherThanReadingLikeTheRest() = runComposeUiTest {
        val sheet = Sheet()
        sheet.caps.value = capsOf(
            SessionInfo("default", running = true),
            SessionInfo("agents", running = true, served = false),
        )
        setContent { Themed { Box(Modifier.size(420.dp, 900.dp)) { sheet.render() } } }

        onNodeWithContentDescription("Named session,", substring = true)
            .performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        onNodeWithContentDescription("default, running").assertExists()
        onNodeWithContentDescription("agents, running, not served by this node").assertExists()
    }

    // Everything else this sheet makes appears behind it, so the ack still closes it. Guards the
    // lazy version of the fix — never closing at all.
    @Test
    fun everyOtherOpStillClosesTheSheetOnTheNodesAck() = runComposeUiTest {
        val sheet = Sheet()
        setContent { Themed { Box(Modifier.size(420.dp, 900.dp)) { sheet.render() } } }

        onNodeWithContentDescription("Create workspace").performClick()
        waitForIdle()
        sheet.acknowledge("workspace.create")
        waitForIdle()

        assertTrue(sheet.dismissed)
        assertEquals(0, sheet.refreshes)
    }

    // A refused session op is the one case where the sheet has something of its own to say.
    @Test
    fun aRefusedSessionOpIsShownRatherThanRefreshedAway() = runComposeUiTest {
        val sheet = Sheet()
        setContent { Themed { Box(Modifier.size(420.dp, 900.dp)) { sheet.render() } } }

        onNodeWithContentDescription("Named session, its own server")
            .performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        onNodeWithContentDescription("name").performTextInput("agents")
        waitForIdle()
        onNodeWithContentDescription("Start session").performClick()
        waitForIdle()
        sheet.outcome.value = ServerMsg.Managed(
            op = "session.create",
            ok = false,
            id = null,
            code = "herdr_unavailable",
            message = "agents was started but never appeared in the session list",
        )
        waitForIdle()

        assertFalse(sheet.dismissed)
        assertEquals(0, sheet.refreshes)
        onNodeWithContentDescription("agents was started but never appeared in the session list")
            .assertExists()
    }
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), content = content)
}

@Composable
private fun Sheet.render() {
    val state: MutableState<ServerMsg.Managed?> = outcome
    NewSheet(
        breakpoint = Breakpoint.Portrait,
        node = HOST,
        pane = null,
        nodes = listOf(HOST),
        caps = caps.value,
        outcome = state.value,
        onManage = { sent += it },
        onNode = {},
        onNodePicker = {},
        onDismiss = { dismissed = true },
        onRefreshCaps = { refreshes += 1 },
    )
}
