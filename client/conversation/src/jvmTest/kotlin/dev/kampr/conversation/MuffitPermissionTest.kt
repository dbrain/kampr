package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals

private const val MUFFIT = "01JNODE/m:s1"

private val INFO = PaneInfo(
    id = MUFFIT,
    nodeId = "01JNODE",
    label = "install ripgrep",
    cwd = "/home/dbrain/dev/kampr",
    agent = "muffit",
    provider = "muffit",
    agentStatus = "blocked",
    rows = 0,
    hasConversation = true,
    converses = true,
)

private val FRAMES = listOf(
    """{"t":"convo","pane":"$MUFFIT","more":false,"turns":[{"id":"m1","role":"user","blocks":[{"t":"md","text":"install ripgrep"}]}]}""",
    """{"t":"pending","pane":"$MUFFIT","source":"transcript","header":"bash",
       "question":"run `cargo install ripgrep` with write access outside the project",
       "options":[{"key":"1","label":"Allow"},{"key":"2","label":"Always allow"},{"key":"3","label":"Deny"}]}""",
)

// docs/18 "Questions and answers": a permission prompt is answered with the option's key, which
// the node posts to muffit as `allow`; it is never keystrokes, so the card may not say it is.
@OptIn(ExperimentalTestApi::class)
class MuffitPermissionTest {
    @Test
    fun aPermissionIsAnsweredWithItsKeyAndNotDescribedAsKeysIntoAPane() = runComposeUiTest {
        val store = KamprStore()
        for (frame in FRAMES) store.accept(requireNotNull(Wire.decode(frame)) { "undecodable: $frame" })
        RecordingIo.sent.clear()
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
                LocalConnectionStatus provides ConnectionStatus.Live("full"),
            ) {
                Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                    ConversationView(store.pane(MUFFIT), INFO, Modifier.fillMaxSize())
                }
            }
        }
        waitForIdle()
        assertEquals(
            0,
            onAllNodesWithText("keys into the pane", substring = true).fetchSemanticsNodes().size,
            "a muffit permission was described as keystrokes",
        )
        onNodeWithContentDescription("Answer 1, Allow").performClick()
        waitForIdle()
        assertEquals(listOf<ClientMsg>(ClientMsg.Answer(MUFFIT, "1")), RecordingIo.sent.filterIsInstance<ClientMsg.Answer>())
    }

    // Its adapter is muffit itself, so a node that has not yet said it converses has still not
    // left it a terminal to fall back to.
    @Test
    fun aSessionNotYetSaidToConverseIsNotSentToATerminal() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
            ) {
                Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                    ConversationView(KamprStore().pane(MUFFIT), INFO.copy(converses = false, hasConversation = false), Modifier.fillMaxSize())
                }
            }
        }
        waitForIdle()
        assertEquals(0, onAllNodesWithContentDescription("terminal view", substring = true).fetchSemanticsNodes().size)
    }
}
