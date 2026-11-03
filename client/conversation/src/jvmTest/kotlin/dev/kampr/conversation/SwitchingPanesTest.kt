package dev.kampr.conversation

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.v2.runComposeUiTest
import dev.kampr.conversation.files.HeldBytes
import dev.kampr.conversation.files.LocalHeldBytes
import dev.kampr.shared.model.ComposerMirror
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.EditKeys
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals

private const val OTHER_ID = "01JNODE.../w3:p5"
private const val REPLY = "Reply to claude"
private val KEYS = EditKeys(back = "\u007f", left = "\u001b[D", right = "\u001b[C", newline = "\n")

private class Typed : PaneIo {
    val sent = mutableListOf<ClientMsg.InputText>()
    override fun send(msg: ClientMsg) {
        if (msg is ClientMsg.InputText) sent += msg
    }
    override fun prefs(paneId: String): PanePrefs = PanePrefs()

    fun into(pane: String) = sent.filter { it.pane == pane }.map { it.text }
}

private data class Away(val name: String, val pane: String, val view: PaneView)

private val AWAY = listOf(
    Away("another pane's conversation", OTHER_ID, PaneView.Conversation),
    Away("another pane split", OTHER_ID, PaneView.Split),
    Away("this pane's terminal", PANE_ID, PaneView.Terminal),
)

// Every way the reply box is put away and built again with the pane's draft: the desktop opens a
// pane from the sidebar into the slot the last pane was in, and a look at the terminal takes the
// box out of the composition altogether.
//
// Unechoed is the node on a re-watch, which published nothing of the pane's box (the reported
// "the conversation pane and the terminal are out of sync"): the box built again took the last
// line it had already heard for the desk emptying the box, and typed what came next on top of
// the words still in the pane.
@OptIn(ExperimentalTestApi::class)
class SwitchingPanesTest {
    @Test
    fun a_reply_survives_the_box_being_built_again() {
        for (away in AWAY) {
            for (echoed in listOf(true, false)) lookAway(away, echoed)
        }
    }

    private fun lookAway(away: Away, echoed: Boolean) = runComposeUiTest {
        val case = "${away.name}, ${if (echoed) "echoed" else "unechoed"}"
        val store = KamprStore()
        listOf(PANE_ID, OTHER_ID).forEach { store.accept(ServerMsg.ConvoComposer(it, null, "\u0003", 0, KEYS)) }
        val showing = mutableStateOf(PANE_ID)
        val view = mutableStateOf(PaneView.Conversation)
        val io = Typed()
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Desk),
                LocalConnectionStatus provides ConnectionStatus.Live("full"),
                LocalPaneIo provides io,
                LocalHeldBytes provides HeldBytes(),
            ) {
                PaneScreenDesktop(
                    pane = store.pane(showing.value),
                    info = demoInfo().copy(id = showing.value),
                    view = view.value,
                    surfaces = ConversationSurfaces(),
                    readOnly = false,
                    onView = {},
                )
            }
        }

        onNodeWithContentDescription(REPLY).performTextInput("hello")
        if (echoed) runOnIdle { store.accept(ServerMsg.ConvoComposer(PANE_ID, "hello", "\u0003", 5, KEYS)) }
        waitForIdle()
        Thread.sleep(ComposerMirror.QUIET + 100)

        showing.value = away.pane
        view.value = away.view
        waitForIdle()
        if (away.pane != PANE_ID) assertEquals("", reply(), "$case: the other pane's box is showing this one's reply")

        showing.value = PANE_ID
        view.value = PaneView.Conversation
        waitForIdle()
        assertEquals("hello", reply(), "$case: the reply is gone on the way back")

        onNodeWithContentDescription(REPLY).performTextInput(" more")
        waitForIdle()
        assertEquals(listOf("hello", " more"), io.into(PANE_ID), case)
        assertEquals(emptyList(), io.into(OTHER_ID), case)
    }

    private fun ComposeUiTest.reply(): String = onNodeWithContentDescription(REPLY).fetchSemanticsNode()
        .config[SemanticsProperties.EditableText].text
}
