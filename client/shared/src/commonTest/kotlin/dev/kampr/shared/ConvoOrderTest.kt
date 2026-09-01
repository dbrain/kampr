package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals

private const val PANE = "01JNODE/w1:p1"

private fun turn(n: Int) = Turn("t$n", if (n % 2 == 0) "user" else "assistant", null, listOf(Block.Md("said $n")))

private fun page(range: IntRange, fresh: Boolean = true) =
    ServerMsg.Convo(pane = PANE, cursor = "t${range.first}", more = true, turns = range.map(::turn), fresh = fresh)

private fun revision(range: IntRange) =
    ServerMsg.ConvoTurn(pane = PANE, sub = null, turns = range.map(::turn))

// Reported off a desk, watching a long session: *"your conversation page on wasm desktop is
// currently showing a very old message"*, then *"now its showing your new messages responding to
// this and my messages but not the messages between"*.
//
// A re-watch is served as a *revision* rather than a page, because a page merges by prepending and
// re-opening a transcript a client already holds pages forwards. That reasoning assumed a revision
// only ever carries turns the client does not have because they are **newer**. It does not: the
// node's page runs back past its own bound to the question that opens the reply it landed in, so a
// re-watch routinely carries turns *older* than the window the client is holding — 64 of them
// against a client holding 40, in the log this came from. Appended to the end, those old turns
// become the newest thing on a view pinned to its own end.
class ConvoOrderTest {
    private fun ids(store: KamprStore) = store.pane(PANE).turns.map { it.id }

    @Test
    fun aRevisionReachingBackPastTheClientsWindowKeepsTheTranscriptInOrder() {
        val store = KamprStore()
        store.accept(page(20..59))
        // The same transcript re-opened: the newest turns, plus the reach back to the question
        // that opens the reply. Nothing has been written since.
        store.accept(revision(10..59))
        assertEquals(
            (10..59).map { "t$it" },
            ids(store),
            "the turns the client had never seen were filed after the ones it had, so the end of " +
                "the transcript — where the view is pinned — became the oldest thing on it",
        )
    }

    @Test
    fun growthStillLandsAtTheEnd() {
        val store = KamprStore()
        store.accept(page(20..59))
        store.accept(revision(58..64))
        assertEquals((20..64).map { "t$it" }, ids(store))
    }

    // A revision with nothing in common with what is drawn — the node has no way to order it from
    // either end, and the turns are the tail it just read. They go last, not first.
    @Test
    fun aRevisionSharingNothingWithWhatIsDrawnIsStillTheNewerHalf() {
        val store = KamprStore()
        store.accept(page(20..59))
        store.accept(revision(70..75))
        assertEquals(((20..59) + (70..75)).map { "t$it" }, ids(store))
    }

    // A launched conversation has both halves for the same reasons, and had the same defect: its
    // first poll after opening hands back everything the file holds, which reaches past the page
    // the reader was given.
    @Test
    fun aLaunchedConversationFilesWhatItGrewByInOrderToo() {
        val store = KamprStore()
        val sub = ServerMsg.Convo(pane = PANE, sub = "s1", cursor = "t20", more = true, turns = (20..29).map(::turn), fresh = true)
        store.accept(sub)
        store.accept(ServerMsg.ConvoTurn(pane = PANE, sub = "s1", turns = (14..31).map(::turn)))
        assertEquals(
            (14..31).map { "t$it" },
            store.pane(PANE).sub("s1").turns.map { it.id },
            "a subagent's earlier steps were filed under the ones that followed them",
        )
    }

    // The whole reason a re-watch is a revision and not a page: a page arriving for a transcript
    // the client is already holding merges by *prepending* what it does not recognise, which is
    // right for `convo.load` and files a newer turn above an hour-old conversation.
    @Test
    fun anOlderPageStillPrepends() {
        val store = KamprStore()
        store.accept(page(20..59))
        store.accept(page(10..19, fresh = false))
        assertEquals((10..59).map { "t$it" }, ids(store))
    }
}
