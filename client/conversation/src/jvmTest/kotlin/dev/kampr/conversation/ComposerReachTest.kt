package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val WRITTEN = "restart the service and tell me what the log says"

private val TALK = ServerMsg.Convo(
    pane = PANE_ID,
    cursor = "c-1",
    more = false,
    turns = listOf(Turn("c-1", "assistant", null, listOf(Block.Md("ready when you are")))),
)

// A reply is `input`, which is `typing`, which is dropped over a socket that is not live — and the
// box clears itself the moment `submit` runs. So the same defect that swallowed an answer swallows
// the operator's own sentence, which is the worse half: an answer can be pressed again, and words
// that have been cleared are gone.
@OptIn(ExperimentalTestApi::class)
class ComposerReachTest {
    @Test
    fun aReplyOverADeadSocketIsNotSentAndIsNotTakenOutOfTheBox() = runComposeUiTest {
        RecordingIo.sent.clear()
        val store = KamprStore()
        store.accept(TALK)
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
                LocalConnectionStatus provides ConnectionStatus.Offline("the node stopped answering", 4_000),
            ) {
                Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                    ConversationView(store.pane(PANE_ID), demoInfo(), Modifier.fillMaxSize())
                }
            }
        }
        waitForIdle()
        // Empty, the box says why it will not send — in the room a placeholder already has, because
        // a rotated phone with the keys up has no line to spare (`ComposerInsetTest`).
        assertTrue(
            onAllNodesWithText("not connected", substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the reply bar said nothing about why it would not send",
        )
        onNodeWithContentDescription("Reply to claude").performTextInput(WRITTEN)
        waitForIdle()
        onNodeWithContentDescription("Send this reply to claude").assertIsNotEnabled()
        assertEquals(emptyList(), RecordingIo.sent.toList(), "a reply left the device over a dead socket")
        assertTrue(
            onAllNodesWithText(WRITTEN, substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the sentence was cleared out of the box and lost",
        )
    }
}
