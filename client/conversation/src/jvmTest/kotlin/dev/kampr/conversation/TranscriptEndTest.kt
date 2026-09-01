package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertTrue

private const val FIRST_LINE = "The opening question, which is off the top of a long transcript."
private const val LAST_LINE = "That paragraph is the end of the newest thing the agent said."

// A real answer is longer than a phone screen, which is the case every fixture here was missing:
// RICH_CONVO ends on a two-line tool card, so a list that stopped at the *top* of its last item
// still looked like it had stopped at the bottom.
private fun tallAnswer(): String =
    (1..40).joinToString("\n\n") { "Paragraph $it of an answer that runs well past one screen." } +
        "\n\n" + LAST_LINE

private const val OLDEST_LINE = "An older page of the same transcript, fetched after the open."

private fun paneWithATallLastTurn(more: Boolean = false): Pair<KamprStore, PaneState> {
    val store = KamprStore()
    store.accept(
        ServerMsg.Convo(
            pane = PANE_ID,
            cursor = "u-1",
            more = more,
            turns = listOf(
                Turn("u-1", "user", "2026-08-23T09:00:00.000Z", listOf(Block.Md(FIRST_LINE))),
                Turn("a-1", "assistant", "2026-08-23T09:00:04.000Z", listOf(Block.Md(tallAnswer()))),
            ),
        )
    )
    return store to store.pane(PANE_ID)
}

private fun olderPage() = ServerMsg.Convo(
    pane = PANE_ID,
    cursor = null,
    more = false,
    turns = listOf(Turn("u-0", "user", "2026-08-23T08:59:00.000Z", listOf(Block.Md(OLDEST_LINE)))),
)

@Composable
private fun Screen(content: @Composable () -> Unit) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
    ) {
        Box(Modifier.fillMaxSize()) { content() }
    }
}

@OptIn(ExperimentalTestApi::class)
class TranscriptEndTest {
    // Reported from a phone: "conversation list seems to scroll to the top / randomly above the
    // bottom when opened". Both halves are the same arithmetic — the transcript opened at the
    // *start* of the last turn instead of the end of the transcript, so how far above the bottom
    // it landed was however tall the newest message happened to be.
    @Test
    fun openingLandsOnTheEndOfTheTranscript() = runComposeUiTest {
        val (_, pane) = paneWithATallLastTurn()
        setContent { Screen { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) } }
        waitForIdle()
        val screen = onRoot().getUnclippedBoundsInRoot()
        assertTrue(
            onAllNodesWithText(LAST_LINE, substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the end of the newest turn was never composed: the transcript opened above it",
        )
        val end = onNodeWithText(LAST_LINE, substring = true).getUnclippedBoundsInRoot()
        assertTrue(
            end.bottom <= screen.bottom,
            "the last line of the transcript ends at ${end.bottom} of ${screen.bottom}",
        )
    }

    // The line that says how far this device has read is an item of its own and it arrives *after*
    // the open — the moment the socket drops, or the reader leaves the pane and comes back. A lazy
    // list anchors on its first visible item, so an item added at the foot is the shape that puts
    // the foot below the fold and leaves it there.
    @Test
    fun theLineAppearingCarriesTheReaderToTheNewEnd() = runComposeUiTest {
        val (store, pane) = paneWithATallLastTurn()
        setContent { Screen { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) } }
        waitForIdle()
        assertTrue(
            onAllNodesWithText("read up to here", substring = true).fetchSemanticsNodes().isEmpty(),
            "the transcript said it was catching up before anything had gone wrong",
        )
        store.noteConversationUnconfirmed(PANE_ID)
        waitForIdle()
        assertTrue(
            onAllNodesWithText("read up to here", substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the socket dropped and the transcript never said how far it had read",
        )
        // Displayed, not merely composed: a lazy list keeps items either side of the fold, and a
        // notice pushed under it is exactly the case this guards.
        onNodeWithText("read up to here", substring = true).assertIsDisplayed()
    }

    // The same open, on a transcript the node is still paging: `more` puts a loading row above
    // turn zero, and every index the scroll is aimed at shifts by one.
    @Test
    fun aPagedTranscriptOpensAtItsEndToo() = runComposeUiTest {
        val (_, pane) = paneWithATallLastTurn(more = true)
        setContent { Screen { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) } }
        waitForIdle()
        assertTrue(
            onAllNodesWithText(LAST_LINE, substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the end of the newest turn was never composed: the transcript opened above it",
        )
        assertTrue(
            onAllNodesWithText(FIRST_LINE, substring = true).fetchSemanticsNodes().isEmpty(),
            "the transcript opened at its top, with the oldest turn on screen",
        )
    }

    // Opening a paged transcript asks for the page before it, so the turns arrive *after* the
    // scroll that put you at the end. Prepending must not move the reader.
    @Test
    fun theOlderPageArrivingLeavesTheReaderAtTheEnd() = runComposeUiTest {
        val (store, pane) = paneWithATallLastTurn(more = true)
        setContent { Screen { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) } }
        waitForIdle()
        store.accept(olderPage())
        waitForIdle()
        assertTrue(
            onAllNodesWithText(LAST_LINE, substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the older page pulled the transcript off its end",
        )
    }
}
