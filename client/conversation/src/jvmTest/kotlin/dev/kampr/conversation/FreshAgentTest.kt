package dev.kampr.conversation

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals

private const val REPLY = "Reply to claude"

private class Recording : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }
    override fun prefs(paneId: String) = PanePrefs()
    override fun show(view: PaneView) = Unit
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.conversation(info: PaneInfo, store: KamprStore = KamprStore()): Recording {
    val io = Recording()
    val pane = store.pane(PANE_ID)
    setContent {
        CompositionLocalProvider(
            LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
            LocalConnectionStatus provides ConnectionStatus.Live("full"),
                LocalPaneIo provides io,
            LocalSafeArea provides SafeArea(top = 0.dp, bottom = 0.dp),
        ) {
            ConversationView(pane, info, Modifier.fillMaxSize())
        }
    }
    waitForIdle()
    return io
}

// The report: *"fresh Claude instance, the conversation tab shows but there's no way to type in
// it"*. Two different questions had been collapsed into one field. `converses` is the node saying
// it serves this harness's transcripts at all — that is what puts the tab there — and
// `has_conversation` is whether it has read one *yet*. A harness that has been launched and not
// yet prompted answers yes and no, and the surface read the second and told the operator this
// node has no adapter for the harness, under a tab that only exists because it has.
//
// Nothing has to be passed through and reattached: a reply *is* a write to the pane's own PTY,
// which is how every reply this app has ever sent has reached an agent. The transcript catches up
// on its own once the harness writes its first record.
@OptIn(ExperimentalTestApi::class)
class FreshAgentTest {
    @Test
    fun anAgentThatHasNotSaidAnythingYetIsStillSomethingYouCanTalkTo() = runComposeUiTest {
        conversation(demoInfo(conversation = false, converses = true, status = "idle"))
        onNodeWithContentDescription(REPLY).assertIsDisplayed()
        onNodeWithText("nothing written down yet", substring = true).assertIsDisplayed()
    }

    @Test
    fun whatIsTypedBeforeTheFirstRecordGoesToThePaneItself() = runComposeUiTest {
        val io = conversation(demoInfo(conversation = false, converses = true, status = "idle"))
        onNodeWithContentDescription(REPLY).performTextInput("run the tests")
        onNodeWithContentDescription("Send this reply to claude").performClick()
        waitForIdle()
        assertEquals(
            listOf<ClientMsg>(ClientMsg.InputText(PANE_ID, "run the tests"), ClientMsg.InputText(PANE_ID, "\r")),
            io.sent.toList(),
            "the first thing said to a fresh agent is typed at its prompt like any other reply",
        )
    }

    // The other road to the same screen, and the one that was reported: *"reusing a terminal that
    // previously ran claude to run claude again — clicking conversation pane I get 'no conversation
    // open for this pane (not_found)' and the conversation pane shows an old conversation"*.
    //
    // The pane had a conversation, the agent was quit and run again, and the node withdrew the
    // session the pane had left. The turns going is not the whole of it: the cursor and `more`
    // the old page was read under outlive them, so the view went on offering "loading earlier
    // turns" against a transcript that is gone — and the `convo.load` behind that offer is what
    // the node answered `not_found`.
    //
    // The mutation that must fail: keep the cursor across a withdrawal, and the offer stands with
    // another `convo.load` behind it.
    @Test
    fun aWithdrawnConversationLeavesThePaneReadyToStartANewOneRatherThanPagingTheOldOne() =
        runComposeUiTest {
            val store = KamprStore()
            store.accept(
                ServerMsg.Convo(
                    pane = PANE_ID, cursor = "a-1", more = true,
                    turns = listOf(proseTurn("a-1", "an answer from the run before")),
                ),
            )
            val io = conversation(
                demoInfo(conversation = false, converses = true, status = "idle"),
                store,
            )
            onNodeWithText("an answer from the run before", substring = true).assertIsDisplayed()
            val askedWhileItWasOpen = io.sent.filterIsInstance<ClientMsg.ConvoLoad>().size

            // The withdrawal: every turn back under its own id, carrying nothing.
            store.accept(
                ServerMsg.ConvoTurn(
                    pane = PANE_ID, sub = null,
                    turns = listOf(Turn("a-1", "assistant", null, emptyList())),
                ),
            )
            waitForIdle()

            onNodeWithText("nothing written down yet", substring = true).assertIsDisplayed()
            onNodeWithContentDescription(REPLY).assertIsDisplayed()
            onNodeWithText("loading earlier turns").assertDoesNotExist()
            assertEquals(
                askedWhileItWasOpen,
                io.sent.filterIsInstance<ClientMsg.ConvoLoad>().size,
                "the pane asked for an older page of a conversation it has been told to let go of",
            )
        }

    // The two panes that genuinely have nothing to read are still told so, and told which of the
    // two reasons it is.
    @Test
    fun aShellIsStillToldItsHistoryIsTheTerminalsOwn() = runComposeUiTest {
        conversation(demoInfo(agent = null, conversation = false, converses = false))
        onNodeWithText("This pane is a shell").assertIsDisplayed()
    }

    @Test
    fun aHarnessThisNodeCannotReadStillSaysSoRatherThanOfferingABoxThatGoesNowhere() = runComposeUiTest {
        conversation(demoInfo(agent = "agy", conversation = false, converses = false))
        onNodeWithText("No transcript for agy").assertIsDisplayed()
    }
}
