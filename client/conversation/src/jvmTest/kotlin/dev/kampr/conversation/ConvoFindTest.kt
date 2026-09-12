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
import dev.kampr.shared.model.LIVE_TURN_ID
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ConvoMatch
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val AT = "2026-08-23T09:00:00.000Z"
private const val NEEDLE = "scrollbar"
private const val DEEP = "a-p2-3"

// What this device holds: the page the conversation opened on, and nothing older.
private fun held(): List<Turn> = (0 until 12).flatMap { n ->
    listOf(
        Turn("u-$n", "user", AT, listOf(Block.Md("question number $n, with words enough for a line"))),
        Turn("a-$n", "assistant", AT, listOf(Block.Md("answer number $n, nothing to find in it"))),
    )
}

// The pages above it, which the node has and this device has never asked for. The conversation
// pages one in by itself the moment it opens at the top, so the hit is put in the one *above* that:
// reaching it has to be the search's own doing rather than something that had already happened.
private fun older(page: Int): List<Turn> = (0 until 6).flatMap { n ->
    listOf(
        Turn("u-p$page-$n", "user", AT, listOf(Block.Md("an older question, $page.$n"))),
        Turn(
            "a-p$page-$n", "assistant", AT,
            listOf(
                Block.Md(
                    if (page == 2 && n == 3) "the deep one: the $NEEDLE column keeps a column back"
                    else "an older answer, $page.$n",
                ),
            ),
        ),
    )
}

private fun storeOfAPage(): KamprStore {
    val store = KamprStore()
    store.accept(ServerMsg.Convo(pane = PANE_ID, cursor = "u-0", more = true, turns = held()))
    return store
}

// A node that answers both halves of this: the transcript search, and the page a hit is in.
private class SearchingIo(private val store: KamprStore, private val total: Int = 41) : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override val searchesTranscript = true
    override fun send(msg: ClientMsg) {
        sent += msg
        when (msg) {
            is ClientMsg.ConvoFind -> store.accept(
                ServerMsg.ConvoFound(
                    pane = PANE_ID,
                    query = msg.query,
                    // Newest first, which is what the node answers with.
                    matches = listOf(
                        ConvoMatch("a-11", "assistant", AT, 0, 1, "answer number 11, nothing to find in it"),
                        ConvoMatch(DEEP, "assistant", AT, 30, 2, "the deep one: the $NEEDLE column keeps a column back"),
                    ),
                    total = total,
                ),
            )
            is ClientMsg.ConvoLoad -> {
                val page = if (msg.before == "u-0") 1 else 2
                store.accept(
                    ServerMsg.Convo(
                        pane = PANE_ID,
                        cursor = "u-p$page-0",
                        more = page < 2,
                        turns = older(page),
                    ),
                )
            }
            else -> Unit
        }
    }
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override fun show(view: PaneView) = Unit
}

// One that promises nothing, which is every node built before the verb existed.
private class OldNodeIo : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override fun show(view: PaneView) = Unit
}

@Composable
private fun Transcript(store: KamprStore, io: PaneIo) {
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
class ConvoFindTest {
    private fun ComposeUiTest.shows(text: String) =
        onAllNodesWithText(text, substring = true).fetchSemanticsNodes().isNotEmpty()

    private fun ComposeUiTest.search(store: KamprStore, io: PaneIo, answered: Boolean = true) {
        setContent { Transcript(store, io) }
        waitForIdle()
        onNodeWithContentDescription("Search the transcript").performClick()
        onNodeWithContentDescription("Search the transcript").performTextInput(NEEDLE)
        waitForIdle()
        // The ask is debounced, so the clock has to run for it. Waited on rather than slept
        // through: what the counter says is a function of the answer having arrived.
        if (answered) {
            waitUntil(timeoutMillis = 4_000) { store.pane(PANE_ID).convoFound != null }
            waitForIdle()
        }
    }

    // The whole point of asking the node: the count is over the session, not over the page this
    // device happens to hold — which holds none of these matches at all.
    @Test
    fun theCountComesFromTheNodeThatHoldsTheTranscript() = runComposeUiTest {
        val store = storeOfAPage()
        val io = SearchingIo(store)
        search(store, io)
        assertEquals(
            listOf(ClientMsg.ConvoFind(PANE_ID, NEEDLE)),
            io.sent.filterIsInstance<ClientMsg.ConvoFind>(),
            "the node is asked once the typing stops",
        )
        // Two listed of forty-one found, and it says both rather than promising forty-one steps.
        // Standing on the newest of the two, which is the nearest to where the reader is.
        onNodeWithContentDescription("Match 2 of 2 listed, 41 in the transcript").assertExists()
        assertTrue(shows("2/2 of 41"), "the tally does not carry both numbers")
    }

    // A hit older than anything this device holds. Reaching it is the `convo.load` walk the reader
    // would have made by scrolling to the top, and the aim lands once the page is in.
    @Test
    fun steppingToAHitThisDeviceDoesNotHoldPagesBackUntilItDoes() = runComposeUiTest {
        val store = storeOfAPage()
        val io = SearchingIo(store)
        search(store, io)
        assertTrue(!shows("the deep one:"), "the deep turn is held before anything asked for it")

        // Backwards from the newest match, which is where the search landed: the deep one is the
        // one before it.
        onNodeWithContentDescription("Previous match").performClick()
        waitUntil(timeoutMillis = 8_000) { shows("the deep one:") }
        waitForIdle()
        assertTrue(
            io.sent.any { it is ClientMsg.ConvoLoad },
            "nothing paged backwards for a hit this device does not hold",
        )
        assertTrue(shows("the deep one:"), "the deep match never arrived on the screen")
    }

    // The list is the other way through the same results, and it aims at the one pressed — which
    // for a turn this device does not hold means paging for it, exactly as stepping does.
    @Test
    fun theListOfMatchesAimsTheTranscriptAtTheOnePressed() = runComposeUiTest {
        val store = storeOfAPage()
        val io = SearchingIo(store)
        search(store, io)
        onNodeWithContentDescription("List the matches").performClick()
        waitForIdle()
        // Oldest first, which is the transcript's own order: the deep one is the top row.
        assertTrue(shows("the deep one:"), "the list does not carry the line each hit matched")
        assertTrue(shows("30 back"), "the list does not say how far back a hit is")

        onNodeWithContentDescription("the deep one:", substring = true).performClick()
        waitForIdle()
        onNodeWithContentDescription("Back to the transcript").assertDoesNotExist()
        assertTrue(shows("the deep one:"), "pressing a result did not put its turn on the screen")
    }

    // A node too old for the verb promises nothing, and the client says what it searched instead of
    // asking for a frame that is never coming.
    @Test
    fun aNodeThatCannotSearchIsNotAskedAndTheCountSaysWhatItCovered() = runComposeUiTest {
        val store = storeOfAPage()
        val io = OldNodeIo()
        search(store, io, answered = false)
        mainClock.advanceTimeBy(2_000)
        waitForIdle()
        assertEquals(
            emptyList(),
            io.sent.filterIsInstance<ClientMsg.ConvoFind>(),
            "a node with no promise was asked anyway",
        )
        onNodeWithContentDescription("No matches in the turns loaded so far").assertExists()
    }

    // The node searched the transcript; the answer being written *now* is not in it. It is read
    // off the screen, it is on the reader's screen too, and a hit in it used to be highlighted and
    // counted by nobody.
    @Test
    fun aHitInTheMessageBeingWrittenNowIsCountedTheNodeCannotHaveSeenIt() = runComposeUiTest {
        val store = storeOfAPage()
        val io = SearchingIo(store)
        search(store, io)
        onNodeWithContentDescription("Match 2 of 2 listed, 41 in the transcript").assertExists()

        // The live preview, which the node publishes off the pane's own screen under one reserved
        // id — not out of the transcript it just searched.
        store.accept(
            ServerMsg.ConvoTurn(
                PANE_ID,
                listOf(Turn(LIVE_TURN_ID, "assistant", null, listOf(Block.Md("and the $NEEDLE again, just now")))),
            ),
        )
        waitForIdle()
        // Counted, and the reader is left standing where they were: a match arriving is not a
        // reason to move anybody (#540). It is the step that goes to it.
        onNodeWithContentDescription("Match 2 of 3 listed, 41 in the transcript").assertExists()
        onNodeWithContentDescription("Next match").performClick()
        waitForIdle()
        onNodeWithContentDescription("Match 3 of 3 listed, 41 in the transcript").assertExists()
        assertTrue(shows("just now"), "stepping to the newest match did not reach the live one")
    }

    // A list of fifty under a search that found two hundred is not the search, and the only thing
    // the reader can do about it is ask something narrower.
    @Test
    fun theListSaysWhenItIsShowingLessThanTheSearchFound() = runComposeUiTest {
        val store = storeOfAPage()
        search(store, SearchingIo(store))
        onNodeWithContentDescription("List the matches").performClick()
        waitForIdle()
        assertTrue(
            shows("41 matches in the transcript"),
            "the list let two matches stand in for forty-one",
        )
        assertTrue(shows("narrow the search"), "and said nothing about what to do")
    }
}
