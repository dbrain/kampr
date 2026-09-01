package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertTrue

private const val NEWEST = "The last thing this device managed to read off the transcript."

private fun paged(): Pair<KamprStore, PaneState> {
    val store = KamprStore()
    store.accept(
        ServerMsg.Convo(
            pane = PANE_ID, cursor = "u-1", more = false,
            turns = listOf(
                Turn("u-1", "user", "2026-08-31T09:00:00.000Z", listOf(Block.Md("which path is dead?"))),
                Turn("a-1", "assistant", "2026-08-31T09:00:04.000Z", listOf(Block.Md(NEWEST))),
            ),
        ),
    )
    return store to store.pane(PANE_ID)
}

@Composable
private fun Screen(status: ConnectionStatus, content: @Composable () -> Unit) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
        LocalConnectionStatus provides status,
    ) {
        Box(Modifier.fillMaxSize()) { content() }
    }
}

// Where the notice goes, which is the whole of the change. It used to be a banner pinned over the
// head of the transcript with the turns washed out underneath it; a reader took that as a
// statement about the words. It is a boundary, so it is drawn at the foot of what was read — under
// the newest turn, in the place a transcript pinned to its own end already puts the reader's eye.
@OptIn(ExperimentalTestApi::class)
class CatchingUpLineTest {
    @Test
    fun theNoticeIsDrawnUnderTheNewestTurnRatherThanOverTheWholeTranscript() = runComposeUiTest {
        val (store, pane) = paged()
        store.noteConversationUnconfirmed(PANE_ID)
        setContent {
            Screen(ConnectionStatus.Live("owner")) {
                ConversationView(pane, demoInfo(status = "idle"), Modifier.fillMaxSize())
            }
        }
        val turn = onNodeWithText(NEWEST, substring = true).getUnclippedBoundsInRoot()
        val notice = onNodeWithText("read up to here", substring = true).getUnclippedBoundsInRoot()
        assertTrue(
            notice.top >= turn.bottom,
            "the notice was drawn at ${notice.top} with the newest turn ending at ${turn.bottom} — " +
                "an edge above what it is the edge of is a banner, not a boundary",
        )
    }

    // A page landed on a live socket: everything drawn is the conversation as it stands, and a
    // line saying otherwise is a lie a reader has to learn to ignore.
    @Test
    fun aConfirmedTranscriptOnALiveSocketDrawsNoLineAtAll() = runComposeUiTest {
        val (_, pane) = paged()
        setContent {
            Screen(ConnectionStatus.Live("owner")) {
                ConversationView(pane, demoInfo(status = "idle"), Modifier.fillMaxSize())
            }
        }
        assertTrue(
            onAllNodesWithText("read up to here", substring = true).fetchSemanticsNodes().isEmpty(),
            "a confirmed transcript on a live socket was told it had only been read so far",
        )
    }
}
