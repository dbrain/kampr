package dev.kampr.terminal

import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.render.LogicalText
import dev.kampr.terminal.render.SurfaceRows
import dev.kampr.terminal.review.HistoryEdge
import dev.kampr.terminal.review.ReviewMove
import dev.kampr.terminal.review.ReviewState
import dev.kampr.terminal.review.ReviewSurface
import dev.kampr.terminal.review.historyEdge
import dev.kampr.terminal.review.historyEdgeSpoken
import dev.kampr.terminal.review.historyWarning
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w3:p1"

private fun pane(cols: Int = 24, rows: Int = 4): PaneState {
    val pane = PaneState(PANE, StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = PANE,
            cols = cols,
            rows = rows,
            rowsData = (0 until rows).map { RowDiff(it, listOf(Run(0, "live row $it"))) },
            cursor = Cursor(0, rows - 1, true),
            links = emptyList(),
        ),
    )
    return pane
}

private fun PaneState.history(
    fromTop: Int,
    lines: List<String>,
    totalRows: Int = lines.size,
    complete: Boolean = true,
    capped: Boolean = false,
) = applyScrollback(
    ServerMsg.Scrollback(
        pane = PANE,
        fromTop = fromTop,
        rows = lines.mapIndexed { index, text -> RowDiff(fromTop + index, listOf(Run(0, text))) },
        totalRows = totalRows,
        complete = complete,
        capped = capped,
    ),
)

private fun surfaceOf(pane: PaneState): ReviewSurface {
    val rows = SurfaceRows(pane)
    return ReviewSurface(rows, LogicalText(rows), rows.historyRows + pane.cursor.row)
}

// The review mode ADR 0010 named as its own missing piece: a reader-owned cursor over the grid,
// moved deliberately, that a live repaint underneath can never silently relocate.
class ReviewTest {
    @Test
    fun reviewWalksTheGridRowByRowFromTheCursor() {
        val pane = pane()
        pane.history(0, listOf("old zero", "old one"))
        val review = ReviewState()
        review.enter(surfaceOf(pane))
        assertTrue(review.active)
        assertTrue(review.utterance.contains("live row 3"), review.utterance)
        assertTrue(review.utterance.contains("of 6"), "entering says where the reader is: ${review.utterance}")

        review.step(surfaceOf(pane), ReviewMove.PreviousLine)
        assertTrue(review.utterance.contains("live row 2"), review.utterance)
        repeat(3) { review.step(surfaceOf(pane), ReviewMove.PreviousLine) }
        assertTrue(review.utterance.contains("old one"), review.utterance)
        review.step(surfaceOf(pane), ReviewMove.NextLine)
        assertTrue(review.utterance.contains("live row 0"), review.utterance)
    }

    // The whole point. History arriving slides the live grid down the surface, so a reader parked
    // on a raw row index ends up somewhere else without being told. Both halves of the surface are
    // anchored against that: history by its absolute ring index, the grid by its own row.
    @Test
    fun aParkedReaderIsNotMovedWhenHistoryGrowsUnderneath() {
        val pane = pane()
        pane.history(0, listOf("first", "second", "third"))
        val review = ReviewState()
        review.enter(surfaceOf(pane))
        review.step(surfaceOf(pane), ReviewMove.PreviousLine)
        review.step(surfaceOf(pane), ReviewMove.PreviousLine)
        assertTrue(review.utterance.contains("live row 1"), review.utterance)
        val onGrid = review.row

        pane.history(0, listOf("first", "second", "third", "fourth", "fifth"), totalRows = 5)
        review.sync(surfaceOf(pane))
        assertEquals(onGrid + 2, review.row, "the surface grew above the reader and the index follows it")
        assertFalse(review.touched, "the row under the reader still says the same thing")
        review.step(surfaceOf(pane), ReviewMove.Reread)
        assertTrue(review.utterance.contains("live row 1"), review.utterance)

        repeat(4) { review.step(surfaceOf(pane), ReviewMove.PreviousLine) }
        assertTrue(review.utterance.contains("third"), review.utterance)
        val inHistory = review.row
        pane.history(0, listOf("first", "second", "third", "fourth", "fifth", "sixth"), totalRows = 6)
        review.sync(surfaceOf(pane))
        assertEquals(inHistory, review.row, "a history row keeps its place when more history lands")
        assertFalse(review.touched)
    }

    // Parked on a live row, the pane repaints it. The reader stays put and is told — on the next
    // thing they hear, not by being spoken over.
    @Test
    fun aRepaintUnderTheReaderIsFlaggedRatherThanSpokenOver() {
        val pane = pane()
        val review = ReviewState()
        review.enter(surfaceOf(pane))
        review.step(surfaceOf(pane), ReviewMove.PreviousLine)
        val parked = review.row
        assertTrue(review.notice.isEmpty(), "nothing has changed yet")

        pane.applyPatch(
            ServerMsg.GridPatch(
                pane = PANE,
                rows = listOf(RowDiff(2, listOf(Run(0, "recompiled")))),
                cursor = null,
                links = emptyList(),
            ),
        )
        review.sync(surfaceOf(pane))
        assertEquals(parked, review.row, "a repaint must not move the review cursor")
        assertTrue(review.touched)
        assertTrue(review.notice.isNotEmpty(), "the reader gets one quiet notice, not a re-read")

        review.step(surfaceOf(pane), ReviewMove.Reread)
        assertTrue(review.utterance.startsWith("Changed."), review.utterance)
        assertTrue(review.utterance.contains("recompiled"), review.utterance)
        assertFalse(review.touched, "reading it clears the flag")
        assertTrue(review.notice.isEmpty())
    }

    @Test
    fun theTopOfTheSurfaceSaysWhereTheRecordEndsAndWhy() {
        val whole = pane().also { it.history(0, listOf("a", "b"), complete = true, capped = false) }
        val clipped = pane().also { it.history(0, listOf("a", "b"), complete = true, capped = true) }
        val discarded = pane().also {
            it.history(1200, listOf("a", "b"), complete = false, capped = true)
        }

        assertEquals(HistoryEdge.Whole, historyEdge(surfaceOf(whole)))
        assertEquals(HistoryEdge.Clipped, historyEdge(surfaceOf(clipped)))
        assertEquals(HistoryEdge.Discarded, historyEdge(surfaceOf(discarded)))

        val review = ReviewState()
        for ((pane, expected) in listOf(
            whole to "starts here",
            clipped to "never captured",
            discarded to "1200 rows",
        )) {
            review.enter(surfaceOf(pane))
            repeat(8) { review.step(surfaceOf(pane), ReviewMove.PreviousLine) }
            assertTrue(review.utterance.contains(expected), "$expected missing from ${review.utterance}")
        }
    }

    // A harness on the alternate screen takes herdr's ring with it (#438), so the node stops
    // vouching for the shell session that ran before it: base advanced, nothing held, `complete`
    // gone and `capped` set — its way of saying "there was more and it cannot be reached".
    //
    // The row count was asked first, so all of that arrived as `None` and the surface told the
    // operator "This pane keeps no history" — the node's careful "I could not reach it" rendered
    // as "there is none", which is #233 one level up from where it usually happens.
    @Test
    fun aPaneWhoseHistoryTheProgramTookSaysSoRatherThanClaimingItNeverHadAny() {
        val superseded = pane().also {
            it.history(0, listOf("a", "b"), complete = true, capped = false)
            it.history(2, emptyList(), totalRows = 0, complete = false, capped = true)
        }
        assertEquals(0, surfaceOf(superseded).historyRows, "the shell era should be gone")
        assertEquals(HistoryEdge.Superseded, historyEdge(surfaceOf(superseded)))
        assertTrue(
            historyWarning(surfaceOf(superseded))!!.contains("its own"),
            "the reader is not told who does have the history",
        )
        assertFalse(
            historyEdgeSpoken(surfaceOf(superseded)).contains("keeps no history"),
            "a pane whose history was taken is not a pane that never had any",
        )
    }

    // The other side of it: a pane that genuinely never had a ring is still `None`, and widening
    // the new state to every empty history would put a notice on every fresh shell pane.
    @Test
    fun aPaneThatNeverHadHistoryIsStillQuietAboutIt() {
        assertEquals(HistoryEdge.None, historyEdge(surfaceOf(pane())))
        assertEquals(null, historyWarning(surfaceOf(pane())))
    }

    // Quiet when the record is whole: a permanent badge on every pane teaches people to ignore it.
    @Test
    fun onlyAnIncompleteRecordWarns() {
        val whole = pane().also { it.history(0, listOf("a")) }
        val clipped = pane().also { it.history(0, listOf("a"), capped = true) }
        val discarded = pane().also { it.history(90, listOf("a"), complete = false, capped = true) }
        val bare = pane()

        assertEquals(null, historyWarning(surfaceOf(whole)))
        assertEquals(null, historyWarning(surfaceOf(bare)), "a pane with no ring has nothing to warn about")
        assertTrue(historyWarning(surfaceOf(clipped))!!.contains("1000"), historyWarning(surfaceOf(clipped))!!)
        assertTrue(historyWarning(surfaceOf(discarded))!!.contains("90"), historyWarning(surfaceOf(discarded))!!)
    }

    @Test
    fun wordsAreWalkedWithinARowAndRollOntoTheNext() {
        val pane = pane(cols = 40)
        pane.history(0, listOf("cargo test -p kampr-term", "ok"))
        val review = ReviewState()
        review.enter(surfaceOf(pane))
        repeat(5) { review.step(surfaceOf(pane), ReviewMove.PreviousLine) }
        assertTrue(review.utterance.contains("cargo test -p kampr-term"), review.utterance)

        review.step(surfaceOf(pane), ReviewMove.NextWord)
        assertTrue(review.utterance.startsWith("cargo"), review.utterance)
        assertTrue(review.utterance.contains("1 of 4"), review.utterance)
        repeat(3) { review.step(surfaceOf(pane), ReviewMove.NextWord) }
        assertTrue(review.utterance.contains("4 of 4"), review.utterance)

        val row = review.row
        review.step(surfaceOf(pane), ReviewMove.NextWord)
        assertEquals(row + 1, review.row, "a word past the end of a row rolls onto the next row")
        assertTrue(review.utterance.startsWith("ok"), review.utterance)

        review.step(surfaceOf(pane), ReviewMove.PreviousWord)
        assertEquals(row, review.row)
        assertTrue(review.utterance.contains("4 of 4"), review.utterance)
    }

    @Test
    fun backToNowReturnsToTheCursorAndLeavingResumesTheLiveLine() {
        val pane = pane()
        pane.history(0, listOf("a", "b", "c"))
        val review = ReviewState()
        review.enter(surfaceOf(pane))
        repeat(4) { review.step(surfaceOf(pane), ReviewMove.PreviousLine) }
        assertTrue(review.row < surfaceOf(pane).cursorIndex)

        review.step(surfaceOf(pane), ReviewMove.Now)
        assertEquals(surfaceOf(pane).cursorIndex, review.row)
        assertTrue(review.utterance.contains("live row 3"), review.utterance)

        review.leave()
        assertFalse(review.active)
        assertTrue(review.utterance.isEmpty(), "leaving must not leave an utterance to be re-spoken")
    }

    // The node discards rather than splicing across a gap, so a row a reader was parked on can
    // genuinely cease to exist — a ring that restarts past everything held, not the ordinary tail
    // that merely continues from it. Relocating them without a word is the one thing not allowed.
    @Test
    fun aDiscardedRowTellsTheReaderRatherThanQuietlyRelocatingThem() {
        val pane = pane()
        pane.history(0, listOf("one", "two", "three", "four"))
        val review = ReviewState()
        review.enter(surfaceOf(pane))
        repeat(5) { review.step(surfaceOf(pane), ReviewMove.PreviousLine) }
        assertTrue(review.utterance.contains("three"), review.utterance)

        pane.history(9, listOf("ten", "eleven"), totalRows = 2, complete = false, capped = true)
        review.sync(surfaceOf(pane))
        assertTrue(review.lost, "the anchored row is gone and the reader has not been told yet")

        review.step(surfaceOf(pane), ReviewMove.Reread)
        assertTrue(review.utterance.contains("discarded"), review.utterance)
        assertFalse(review.lost)
    }
}
