package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertTrue

private const val AT = "2026-08-23T09:00:00.000Z"
private const val NEEDLE = "scrollbar"
private const val HITS = 6

private fun longTranscript(more: Boolean = false): KamprStore {
    val turns = mutableListOf<Turn>()
    for (n in 0 until 12) {
        turns += Turn("u-$n", "user", AT, listOf(Block.Md("question number $n, and a few more words to take a line")))
        val said =
            if (n % 2 == 0) "hit $n: the $NEEDLE column is the one it keeps back"
            else "answer number $n, nothing to find in it"
        turns += Turn("a-$n", "assistant", AT, listOf(Block.Md(said)))
    }
    val store = KamprStore()
    store.accept(ServerMsg.Convo(pane = PANE_ID, cursor = "u-0", more = more, turns = turns))
    return store
}

private fun pageOf(n: Int): List<Turn> = (0 until 6).flatMap { i ->
    listOf(
        Turn("u-p$n-$i", "user", AT, listOf(Block.Md("older question $n.$i, with enough words to take a line"))),
        Turn("a-p$n-$i", "assistant", AT, listOf(Block.Md("older answer $n.$i, nothing to find in it at all"))),
    )
}

// A transcript that pages backwards while the reader is in it, which every real one does: the
// prepended page moves every row index the search is aiming at.
private class PagingIo(private val store: KamprStore) : PaneIo {
    private var pages = 0
    override fun send(msg: ClientMsg) {
        if (msg !is ClientMsg.ConvoLoad) return
        pages++
        val page = pageOf(pages)
        store.accept(ServerMsg.Convo(pane = PANE_ID, cursor = page.first().id, more = pages < 3, turns = page))
    }
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override fun show(view: PaneView) = Unit
}

@Composable
private fun Transcript(store: KamprStore, io: PaneIo = RecordingIo) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides io,
    ) {
        Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
            ConversationView(store.pane(PANE_ID), demoInfo(), Modifier.fillMaxSize())
        }
    }
}

@OptIn(ExperimentalTestApi::class)
class SearchStepTest {
    private fun ComposeUiTest.shows(text: String) =
        onAllNodesWithText(text, substring = true).fetchSemanticsNodes().isNotEmpty()

    private fun ComposeUiTest.search(store: KamprStore, io: PaneIo = RecordingIo) {
        setContent { Transcript(store, io) }
        waitForIdle()
        onNodeWithContentDescription("Search the transcript").performClick()
        onNodeWithContentDescription("Search the transcript").performTextInput(NEEDLE)
        waitForIdle()
        // The search lands on the match nearest the end the reader is standing at, which is the
        // newest one: a transcript is read from its end.
        onNodeWithContentDescription("Match $HITS of $HITS", substring = true).assertExists()
        assertTrue(shows("hit 10:"), "the nearest match is not on the screen")
    }

    // Backwards through the transcript, which is the direction a search runs from its end.
    private fun ComposeUiTest.back(to: Int) {
        onNodeWithContentDescription("Previous match").performClick()
        waitForIdle()
        onNodeWithContentDescription("Match $to of $HITS", substring = true).assertExists()
        assertTrue(shows("hit ${(to - 1) * 2}:"), "match $to is not on the screen")
    }

    private fun ComposeUiTest.tick(store: KamprStore, id: String) {
        store.accept(
            ServerMsg.ConvoTurn(PANE_ID, listOf(Turn(id, "assistant", AT, listOf(Block.Md("one more line"))))),
        )
        waitForIdle()
    }

    @Test
    fun steppingThroughTheMatchesPutsEachOneOnTheScreen() = runComposeUiTest {
        search(longTranscript())
        for (to in HITS - 1 downTo 1) back(to)

        // And the step off the oldest comes round to the newest rather than stopping on it.
        onNodeWithContentDescription("Previous match").performClick()
        waitForIdle()
        onNodeWithContentDescription("Match $HITS of $HITS", substring = true).assertExists()
        assertTrue(shows("hit 10:"), "stepping past the oldest match did not come round")
    }

    @Test
    fun steppingWorksWhileTheTranscriptIsStillPagingBackwards() = runComposeUiTest {
        val store = longTranscript(more = true)
        search(store, PagingIo(store))
        for (to in HITS - 1 downTo 1) back(to)
    }

    // The pane an operator searches is a pane an agent is writing in, and the transcript follows
    // its own end. A reader who has gone looking for something has left that end, and nothing
    // else says so: a programmatic aim is not a scroll anything reports (#540).
    @Test
    fun aTranscriptTickLeavesTheReaderOnTheMatchTheyAimedAt() = runComposeUiTest {
        val store = longTranscript()
        search(store)
        tick(store, "a-tick-1")
        assertTrue(shows("hit 10:"), "a transcript tick took the reader off the match")

        back(HITS - 1)
        tick(store, "a-tick-2")
        assertTrue(shows("hit 8:"), "a transcript tick took the reader off the match they stepped to")
    }

    // The count is over the turns this client holds, which on a long transcript is the newest page
    // and whatever the reader has paged back to. It says so for as long as that is true.
    @Test
    fun theCountSaysSoWhileThereAreOlderTurnsItHasNotSeen() = runComposeUiTest {
        setContent { Transcript(longTranscript(more = true)) }
        waitForIdle()
        onNodeWithContentDescription("Search the transcript").performClick()
        onNodeWithContentDescription("Search the transcript").performTextInput(NEEDLE)
        waitForIdle()
        onNodeWithContentDescription("Match $HITS of $HITS in the turns loaded so far").assertExists()
        assertTrue(shows("$HITS/$HITS so far"), "the tally does not say what it has searched")
    }

    @Test
    fun theCountIsPlainOnceTheWholeTranscriptIsHere() = runComposeUiTest {
        search(longTranscript())
        onNodeWithContentDescription("Match $HITS of $HITS").assertExists()
        assertTrue(shows("$HITS/$HITS"), "the tally is missing")
    }

    // What the bar itself says about where the reader is standing. A reader the search has moved
    // is not at the end any more, so the way back to it is the control that stands there — and
    // its absence is the transcript still believing the reader never left, which is the defect
    // above seen from the other side.
    @Test
    fun aSearchThatMovedTheReaderLeavesTheWayBackToTheEnd() = runComposeUiTest {
        search(longTranscript())
        onNodeWithContentDescription("Close search").performClick()
        waitForIdle()
        onNodeWithContentDescription("Go to the end of the transcript").assertExists()
    }
}
