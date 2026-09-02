package dev.kampr.shared

import dev.kampr.shared.model.ScrollbackStore
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

// A harness that takes the alternate screen takes herdr's ring with it — measured of Claude Code,
// which sends `?1049h` and never `3J`, so the shell session that ran before it comes back verbatim
// on exit and not one row of the conversation ever enters the ring. The node stops vouching for
// that shell era as this pane's history and says so: base advanced by what it dropped, nothing
// held, `complete` gone and `capped` set.
//
// This half is the client believing it. `apply` is built to never shrink what it holds — a delta
// carries only new rows and the node re-bases each one onto the client's known end — so a
// supersede lands *exactly* on that end and read as an ordinary empty tail changed nothing.
//
// It is not cosmetic. `TerminalView` gives the wheel to the pane only while `historyRows == 0`
// (`scrollToPane`), because a pane Kampr holds history for is one Kampr scrolls itself. So a
// client still holding 363 dead shell rows scrolls those instead of handing the wheel to Claude —
// which is the whole of "it feels like it's lost that I'm scrolling a claude instance".
private fun history(fromTop: Int, count: Int, total: Int = count) = ServerMsg.Scrollback(
    pane = "p",
    fromTop = fromTop,
    rows = (0 until count).map { RowDiff(fromTop + it, listOf(Run(0, "shell line $it"))) },
    totalRows = total,
    complete = fromTop == 0,
    capped = false,
)

private fun superseded(at: Int) = ServerMsg.Scrollback(
    pane = "p",
    fromTop = at,
    rows = emptyList(),
    totalRows = 0,
    complete = false,
    capped = true,
)

class SupersededHistoryTest {
    @Test
    fun aRingThatHoldsNothingTakesTheShellEraWithIt() {
        val store = ScrollbackStore()
        store.apply(history(fromTop = 0, count = 363))
        assertEquals(363, store.historyRows, "the shell era should be held before the harness takes the screen")

        store.apply(superseded(at = 363))

        assertEquals(0, store.historyRows, "the pane went on offering a dead shell session as its history")
        assertEquals(363, store.fromTop)
        assertFalse(store.complete, "an unreachable history is not a complete one")
        assertTrue(store.capped, "the rows are gone and the client has to be able to say why")
        assertEquals(null, store.row(0), "a superseded row is not still readable")
    }

    // The guard the store was built with, and the one this must not spend: a tail carries only the
    // rows it grew by and re-bases onto the client's end, so an ordinary one still stitches.
    @Test
    fun anOrdinaryTailStillStitchesRatherThanDiscarding() {
        val store = ScrollbackStore()
        store.apply(history(fromTop = 0, count = 10))
        store.apply(history(fromTop = 10, count = 5, total = 15))
        assertEquals(15, store.historyRows, "a tail discarded the history it was extending")
        assertEquals(0, store.fromTop)
    }

    // A poll that found nothing new is not a poll that found nothing: the node reports the ring's
    // whole depth every time, so only a document claiming *no rows at all* is a discard.
    @Test
    fun aPollWithNoNewRowsIsNotADiscard() {
        val store = ScrollbackStore()
        store.apply(history(fromTop = 0, count = 363))
        store.apply(
            ServerMsg.Scrollback(
                pane = "p",
                fromTop = 0,
                rows = emptyList(),
                totalRows = 363,
                complete = true,
                capped = false,
            )
        )
        assertEquals(363, store.historyRows, "a quiet poll threw the operator's history away")
    }

    @Test
    fun aPaneThatNeverHadHistoryIsUndisturbedByOne() {
        val store = ScrollbackStore()
        store.apply(superseded(at = 0))
        assertEquals(0, store.historyRows)
        assertEquals(0, store.fromTop)
    }
}
