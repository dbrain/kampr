package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.platform.PickedFile
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val ATTACHED = PickedFile("shot.png", "image/png", byteArrayOf(1, 2, 3, 4, 5))
private const val SAID = "shot.png is on claude's machine, and its path is typed in"
private const val REFUSED = "that is larger than this node will take"

private class Sink : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
}

private fun freshPane(): PaneState = KamprStore().pane(PANE_ID)

@Composable
private fun Screen(pane: PaneState, io: Sink, handover: MutableState<Handover>) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalConnectionStatus provides ConnectionStatus.Live("full"),
        LocalPaneIo provides io,
    ) {
        Box(Modifier.fillMaxSize()) {
            ConversationView(pane, demoInfo(), Modifier.fillMaxSize(), handover = handover)
        }
    }
}

// Reported from a phone: a screenshot was attached and sent, and the green line over the reply box
// went on saying its path was typed in. It was a statement about a draft that had already gone, and
// nothing ever took it down — the state had no transition back to idle at all.
@OptIn(ExperimentalTestApi::class)
class HandoverLineTest {
    @Test
    fun a_sent_reply_takes_down_the_line_about_the_draft_it_just_carried() = runComposeUiTest {
        val io = Sink()
        val pane = freshPane()
        val handover = mutableStateOf<Handover>(Handover.Idle)
        setContent { Screen(pane, io, handover) }

        runOnIdle { handover.value = handoverOf(pane, io, ATTACHED) }
        waitForIdle()
        onNodeWithText(SAID).assertExists()

        onNodeWithContentDescription("Reply to claude").performTextInput("have a look at that")
        onNodeWithContentDescription("Send this reply to claude").performClick()
        waitForIdle()

        assertEquals(
            listOf("have a look at that", "\r"),
            io.sent.filterIsInstance<ClientMsg.InputText>().map { it.text },
            "the reply did not go, so nothing here is about the line clearing",
        )
        assertTrue(
            onAllNodesWithText(SAID, substring = true).fetchSemanticsNodes().isEmpty(),
            "the line still says the path is typed in, over a box the reply has left",
        )
    }

    // A refusal is the node's only report that a file never arrived, and it is quiet everywhere
    // else by design. An error nobody has read is not something to sweep away with a send that had
    // nothing to do with it — the next handover clears it, which is the press that fixes it anyway.
    @Test
    fun a_refusal_outlives_a_send_because_it_is_not_about_the_draft() = runComposeUiTest {
        val io = Sink()
        val pane = freshPane()
        val handover = mutableStateOf<Handover>(Handover.Refused(REFUSED))
        setContent { Screen(pane, io, handover) }

        onNodeWithText(REFUSED).assertExists()
        onNodeWithContentDescription("Reply to claude").performTextInput("never mind")
        onNodeWithContentDescription("Send this reply to claude").performClick()
        waitForIdle()

        onNodeWithText(REFUSED).assertExists()
    }

    @Test
    fun the_transition_the_state_never_had() {
        assertEquals(Handover.Idle, handoverAfterSend(Handover.Sent("shot.png")))
        assertEquals(Handover.Idle, handoverAfterSend(Handover.Going("shot.png")))
        assertEquals(Handover.Idle, handoverAfterSend(Handover.Idle))
        assertEquals(Handover.Refused(REFUSED), handoverAfterSend(Handover.Refused(REFUSED)))
    }
}
