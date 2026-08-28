package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.ClientMsg
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
