package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val HANDLE = "aGFuZGxl"
private const val DEEPER = "ZGVlcGVy"

private const val LAUNCHING = """{"t":"convo","pane":"01JNODE.../w3:p2","cursor":"u1","more":false,"turns":[
    {"id":"u1","role":"user","blocks":[{"b":"md","text":"map the manage op path"}]},
    {"id":"t1","role":"assistant","blocks":[
      {"b":"tool","name":"Agent","summary":"Explore","state":"done"},
      {"b":"sub","id":"aGFuZGxl","kind":"Explore","title":"Map the manage op end-to-end path","depth":1}]}]}"""

private const val LAUNCHED = """{"t":"convo","pane":"01JNODE.../w3:p2","sub":"aGFuZGxl","fresh":true,"more":false,
    "turns":[
    {"id":"s1","role":"user","blocks":[{"b":"md","text":"map the manage op path end to end"}]},
    {"id":"s2","role":"assistant","blocks":[
      {"b":"md","text":"Six hops, and the fourth is the one that drops it."},
      {"b":"tool","name":"Agent","summary":"Explore","state":"done"},
      {"b":"sub","id":"ZGVlcGVy","kind":"general-purpose","title":"Read the mesh relay","depth":2}]}]}"""

private const val DEEPEST = """{"t":"convo","pane":"01JNODE.../w3:p2","sub":"ZGVlcGVy","fresh":true,"more":false,
    "turns":[{"id":"d1","role":"assistant","blocks":[{"b":"md","text":"The relay drops it on a closed link."}]}]}"""

private const val CARD = "Open the conversation with Explore — Map the manage op end-to-end path"
private const val DEEPER_CARD = "Open the conversation with general-purpose — Read the mesh relay"
private const val BACK = "Back to the pane's own transcript"

private fun pane(vararg frames: String): Pair<KamprStore, PaneState> {
    val store = KamprStore()
    for (frame in frames) store.accept(requireNotNull(Wire.decode(frame)) { "undecodable: $frame" })
    return store to store.pane(PANE_ID)
}

private class SubIo : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }
    override fun prefs(paneId: String) = PanePrefs()
    override fun show(view: PaneView) = Unit
}

@Composable
private fun Screen(pane: PaneState, io: PaneIo, status: ConnectionStatus) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides io,
        LocalConnectionStatus provides status,
    ) {
        Box(Modifier.fillMaxSize()) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) }
    }
}

@Composable
private fun Screen(pane: PaneState) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
    ) {
        Box(Modifier.fillMaxSize()) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) }
    }
}

// The operator's ask, verbatim: *see what the agent is doing by selecting it.*
@OptIn(ExperimentalTestApi::class)
class SubConversationSurfaceTest {
    // **A follow is per-socket.** The node holds it on the pane handle the watch built, and a
    // reconnect builds a new one empty — so unless something re-announces it, the panel keeps the
    // turns it had and never takes another while the subagent goes on working. The whole reason to
    // open one is to watch it work.
    //
    // Nothing did: a sub is asked for when the reader opens it and never again, and the cached sub
    // survives a reconnect. The parent conversation catching up normally is what made that read as
    // the subagent having stopped rather than as the view being dead — every other surface
    // answering correctly, which is the shape #233 taught this project to fear.
    //
    // Re-asking is free by the rule the open path already relies on: the page is `fresh`, so a
    // second ask is how a running subagent's latest step arrives.
    //
    // The mutation that must fail: drop the re-announce, and the second ask never goes.
    @Test
    fun aSubagentTheReaderHadOpenIsFollowedAgainAfterAReconnect() = runComposeUiTest {
        val (_, pane) = pane(LAUNCHING, LAUNCHED)
        val io = SubIo()
        var status: ConnectionStatus by mutableStateOf(ConnectionStatus.Live("full"))
        setContent { Screen(pane, io, status) }
        waitForIdle()
        onNodeWithContentDescription(CARD).performClick()
        waitForIdle()
        assertEquals(
            1,
            io.sent.count { it is ClientMsg.ConvoSub },
            "opening a subagent did not ask the node to follow it: ${io.sent}",
        )

        status = ConnectionStatus.Offline("the wifi went", 1_000)
        waitForIdle()
        status = ConnectionStatus.Live("full")
        waitForIdle()

        assertEquals(
            2,
            io.sent.count { it is ClientMsg.ConvoSub },
            "the follow died with the socket and nothing asked for it again, so this panel would " +
                "never take another turn: ${io.sent}",
        )
    }

    @Test
    fun aTurnThatLaunchedAnAgentOffersToOpenItByKindAndTitle() = runComposeUiTest {
        val (_, pane) = pane(LAUNCHING)
        setContent { Screen(pane) }
        onNodeWithText("Explore — Map the manage op end-to-end path").assertExists()
        onNodeWithContentDescription(CARD).assertExists()
    }

    @Test
    fun openingOneAsksTheNodeForItByTheHandleTheBlockCarried() = runComposeUiTest {
        RecordingIo.sent.clear()
        val (_, pane) = pane(LAUNCHING)
        setContent { Screen(pane) }
        onNodeWithContentDescription(CARD).performClick()
        waitForIdle()
        assertEquals(
            listOf(ClientMsg.ConvoSub(PANE_ID, HANDLE, null)),
            RecordingIo.sent.filterIsInstance<ClientMsg.ConvoSub>(),
        )
    }

    // The one thing that must not happen: a launched conversation shown as the pane's own reply.
    // Its page is `fresh`, so a client that routed it into the pane's turns would clear the
    // transcript and put another agent's words there under this agent's name.
    @Test
    fun aLaunchedConversationIsReadAsItsOwnAndTheTranscriptIsStillThereBehindIt() = runComposeUiTest {
        val (store, pane) = pane(LAUNCHING)
        setContent { Screen(pane) }
        onNodeWithContentDescription(CARD).performClick()
        waitForIdle()
        store.accept(requireNotNull(Wire.decode(LAUNCHED)))
        waitForIdle()
        onNodeWithText("Six hops, and the fourth is the one that drops it.").assertExists()
        assertTrue(
            onAllNodesWithText("map the manage op path", substring = false).fetchSemanticsNodes().isEmpty(),
            "the pane's own question was still on screen under the conversation it launched",
        )
        onNodeWithContentDescription(BACK).performClick()
        waitForIdle()
        onNodeWithText("map the manage op path").assertExists()
        assertEquals(listOf("u1", "t1"), pane.turns.map { it.id })
    }

    // `depth` says a launched conversation can launch one of its own, so going in twice and
    // coming back once has to mean something.
    @Test
    fun aConversationLaunchedInsideOneIsOpenedAndLeftOneLevelAtATime() = runComposeUiTest {
        RecordingIo.sent.clear()
        val (store, pane) = pane(LAUNCHING)
        setContent { Screen(pane) }
        onNodeWithContentDescription(CARD).performClick()
        waitForIdle()
        store.accept(requireNotNull(Wire.decode(LAUNCHED)))
        waitForIdle()
        onNodeWithContentDescription(DEEPER_CARD).performClick()
        waitForIdle()
        store.accept(requireNotNull(Wire.decode(DEEPEST)))
        waitForIdle()
        onNodeWithText("The relay drops it on a closed link.").assertExists()
        assertEquals(
            listOf(ClientMsg.ConvoSub(PANE_ID, HANDLE, null), ClientMsg.ConvoSub(PANE_ID, DEEPER, null)),
            RecordingIo.sent.filterIsInstance<ClientMsg.ConvoSub>(),
        )
        onNodeWithContentDescription(BACK).performClick()
        waitForIdle()
        onNodeWithText("Six hops, and the fourth is the one that drops it.").assertExists()
        onNodeWithContentDescription(BACK).performClick()
        waitForIdle()
        onNodeWithText("map the manage op path").assertExists()
    }
}
