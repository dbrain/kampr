package dev.kampr.conversation

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals

private const val REPLY = "Reply to claude"
private const val SEND = "Send this reply to claude"
private const val KEYS = """"keys":{"back":"\u007f","left":"\u001b[D","right":"\u001b[C","newline":"\n"}"""

private fun composer(text: String?, keys: Boolean = true): String {
    val said = text?.let { "\"$it\"" } ?: "null"
    val caret = text?.length ?: 0
    val measured = if (keys) ""","caret":$caret,$KEYS""" else ""
    return """{"t":"convo.composer","pane":"$PANE_ID","text":$said,"clear":"\u0003"$measured}"""
}

private fun KamprStore.hears(frame: String) = accept(requireNotNull(Wire.decode(frame)))

private fun paneWithNoQuestion(): Pair<KamprStore, PaneState> {
    val (store, pane) = demoPane(RICH_CONVO)
    store.accept(ServerMsg.Pending(PANE_ID, null, emptyList(), "screen"))
    return store to pane
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.box(): String =
    onNodeWithContentDescription(REPLY).fetchSemanticsNode().config[SemanticsProperties.EditableText].text

private fun typedIntoThePane(): List<String> =
    RecordingIo.sent.filterIsInstance<ClientMsg.InputText>().map { it.text }

// The reply box and the pane's own box are one line: what the desk types is in the reply box as it
// is typed, what the reply box types is in the pane as it is typed, and sending is the Enter.
@OptIn(ExperimentalTestApi::class)
class OneLineTest {
    // Taken up, not written: a line typed at the desk fills the box on this screen and nothing goes
    // back to the pane for it (rule 3). The strip that used to stand beside a line the box could
    // not hold has nothing to say about one the box does.
    @Test
    fun theDesksLineIsInTheReplyBoxAndNothingIsWrittenToGetIt() = runComposeUiTest {
        val (store, pane) = paneWithNoQuestion()
        store.hears(composer("push the branch"))
        RecordingIo.sent.clear()
        setContent { Harnessed(pane) }
        waitForIdle()
        assertEquals("push the branch", box())
        assertEquals(emptyList(), RecordingIo.sent.toList(), "looking at the pane wrote to it")
        onAllNodesWithText("added to the end", substring = true).assertCountEquals(0)

        store.hears(composer("push the branch now"))
        waitForIdle()
        assertEquals("push the branch now", box(), "the box stopped following the desk")
    }

    // What the reply box types is typed into the pane as it is typed, and the send is only the
    // Enter — the words are already there, and sending them again would submit them twice.
    @Test
    fun whatTheReplyBoxTypesIsInThePaneAndSendingIsTheEnter() = runComposeUiTest {
        val (store, pane) = paneWithNoQuestion()
        store.hears(composer(null))
        RecordingIo.sent.clear()
        setContent { Harnessed(pane) }
        waitForIdle()
        onNodeWithContentDescription(REPLY).performTextInput("push")
        waitForIdle()
        assertEquals(listOf("push"), typedIntoThePane())
        onNodeWithContentDescription(SEND).performClick()
        waitForIdle()
        assertEquals(listOf("push", "\r"), typedIntoThePane())
    }

    // **A digit answers a dialog** (#413, #421, #487): a question standing on the pane is a box
    // nothing is typed into, and the words wait in this one until they are sent.
    @Test
    fun nothingIsTypedIntoAPaneWithAQuestionStandingOnIt() = runComposeUiTest {
        val (store, pane) = demoPane(RICH_CONVO)
        store.hears(composer(null))
        RecordingIo.sent.clear()
        setContent { Harnessed(pane) }
        waitForIdle()
        onNodeWithContentDescription(REPLY).performTextInput("1")
        waitForIdle()
        assertEquals(emptyList(), typedIntoThePane())
    }

    // A harness nobody has measured the keys of keeps the box it always had: the line beside it,
    // the reply added to the end of it on send, and nothing typed before then.
    @Test
    fun aHarnessWithNoMeasuredKeysKeepsTheBoxItAlwaysHad() = runComposeUiTest {
        val (store, pane) = paneWithNoQuestion()
        store.hears(composer("push the branch", keys = false))
        RecordingIo.sent.clear()
        setContent { Harnessed(pane) }
        waitForIdle()
        assertEquals("", box())
        onAllNodesWithText("added to the end", substring = true).assertCountEquals(1)
        onNodeWithContentDescription(REPLY).performTextInput(" when")
        waitForIdle()
        assertEquals(emptyList(), typedIntoThePane())
        onNodeWithContentDescription(SEND).performClick()
        waitForIdle()
        assertEquals(listOf(" when", "\r"), typedIntoThePane())
    }
}

@Composable
private fun Harnessed(pane: PaneState) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
        LocalConnectionStatus provides ConnectionStatus.Live("full"),
    ) {
        ConversationView(pane, demoInfo(status = "idle"), Modifier.fillMaxSize())
    }
}
