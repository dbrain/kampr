package dev.kampr.conversation

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithText
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Wire
import kotlin.test.Test

private const val WAITING = "push the branch please"

private const val QUEUE =
    """{"t":"convo.facets","pane":"$PANE_ID","facets":{"queued":[{"text":"$WAITING"}]}}"""

private object ReadOnlyIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override fun show(view: PaneView) = Unit
    override val readOnly: Boolean get() = true
}

@OptIn(ExperimentalTestApi::class)
class QueuedScreenTest {
    // The report, end to end: a message sent while the agent is mid-turn is queued by the harness
    // and written down only when it gets there — minutes, on a long turn — so the terminal pane
    // showed the text and the conversation showed nothing at all.
    @Test
    fun aPromptTheHarnessHasQueuedIsOnTheConversationBeforeAnyRecordArrives() = runComposeUiTest {
        val (store, pane) = demoPane(RICH_CONVO)
        store.accept(requireNotNull(Wire.decode(QUEUE)))
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
            ) {
                ConversationView(pane, demoInfo(status = "working"), Modifier.fillMaxSize())
            }
        }
        onNodeWithText(WAITING).assertIsDisplayed()
        // And it says it is still waiting, which is the whole of what the operator is asking.
        onNodeWithText("queued", ignoreCase = true).assertIsDisplayed()
    }

    // The queue is the pane's state and not this client's own, so a device that may not type still
    // sees what is waiting — including prompts sent from the desk.
    @Test
    fun aReadOnlyDeviceStillSeesWhatIsQueued() = runComposeUiTest {
        val (store, pane) = demoPane(RICH_CONVO)
        store.accept(requireNotNull(Wire.decode(QUEUE)))
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides ReadOnlyIo,
            ) {
                ConversationView(pane, demoInfo(status = "working"), Modifier.fillMaxSize())
            }
        }
        onNodeWithText(WAITING).assertIsDisplayed()
        onAllNodesWithText("read-only device").assertCountEquals(1)
    }
}
