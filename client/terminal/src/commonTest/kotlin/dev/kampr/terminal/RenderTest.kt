package dev.kampr.terminal

import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.render.ModeSelector
import dev.kampr.terminal.render.RenderMode
import dev.kampr.terminal.render.SurfaceRows
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private fun pane(cols: Int = 10, rows: Int = 3): PaneState {
    val pane = PaneState("n/w1:p1", StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = pane.id,
            cols = cols,
            rows = rows,
            rowsData = (0 until rows).map { RowDiff(it, listOf(Run(0, "live$it"))) },
            cursor = Cursor(0, rows - 1, true),
            links = emptyList(),
        ),
    )
    return pane
}

private fun readRow(rows: SurfaceRows, index: Int): String {
    val chars = CharArray(rows.cols)
    val styles = IntArray(rows.cols)
    assertTrue(rows.into(index, chars, styles), "row $index should exist")
    return chars.concatToString().trimEnd()
}

class RenderTest {
    @Test
    fun scrollbackAndTheLiveGridAreOneIndexSpace() {
        val pane = pane()
        pane.applyScrollback(
            ServerMsg.Scrollback(
                pane = pane.id,
                fromTop = 0,
                rows = (0 until 4).map { RowDiff(it, listOf(Run(0, "old$it"))) },
                totalRows = 4,
                complete = true,
                capped = false,
            ),
        )
        val rows = SurfaceRows(pane)
        assertEquals(7, rows.total)
        assertEquals("old0", readRow(rows, 0))
        assertEquals("old3", readRow(rows, 3))
        assertEquals("live0", readRow(rows, 4))
        assertEquals("live2", readRow(rows, 6))
    }

    // total_rows is a depth, not a highest index: from_top only advances, on a node-side discard.
    @Test
    fun historyIsAddressedByAbsoluteRingIndex() {
        val pane = pane()
        pane.applyScrollback(
            ServerMsg.Scrollback(
                pane = pane.id,
                fromTop = 1500,
                rows = listOf(RowDiff(1500, listOf(Run(0, "top"))), RowDiff(1501, listOf(Run(0, "next")))),
                totalRows = 2,
                complete = false,
                capped = true,
            ),
        )
        val rows = SurfaceRows(pane)
        assertEquals(5, rows.total)
        assertEquals("top", readRow(rows, 0))
        assertEquals("next", readRow(rows, 1))
        assertEquals("live0", readRow(rows, 2))
    }

    // What the node actually puts on the wire: `send_history` re-bases every delta's `from_top`
    // onto the client's known end, so an ordinary tail always arrives with from_top advanced.
    // Reading that as a discard collapses the surface under a reader who is scrolled back.
    private fun scrollback(pane: PaneState, from: Int, count: Int, label: String) {
        pane.applyScrollback(
            ServerMsg.Scrollback(
                pane = pane.id,
                fromTop = from,
                rows = (0 until count).map { RowDiff(from + it, listOf(Run(0, "$label$it"))) },
                totalRows = count,
                complete = false,
                capped = true,
            ),
        )
    }

    @Test
    fun aContiguousTailAppendsInsteadOfDiscardingTheHistoryAboveIt() {
        val pane = pane()
        scrollback(pane, from = 1463, count = 90, label = "old")
        assertEquals(93, SurfaceRows(pane).total)

        scrollback(pane, from = 1553, count = 2, label = "new")

        val rows = SurfaceRows(pane)
        assertEquals(95, rows.total, "a two-row tail must lengthen the surface, not truncate it")
        assertEquals(1463, rows.fromTop, "the ring start does not move on a contiguous tail")
        assertEquals("old0", readRow(rows, 0))
        assertEquals("old89", readRow(rows, 89))
        assertEquals("new0", readRow(rows, 90))
        assertEquals("live0", readRow(rows, 92))
    }

    @Test
    fun aTailBeyondTheHeldRangeIsARestartAndDropsWhatCameBefore() {
        val pane = pane()
        scrollback(pane, from = 1463, count = 90, label = "old")

        scrollback(pane, from = 1600, count = 2, label = "new")

        val rows = SurfaceRows(pane)
        assertEquals(5, rows.total, "a gap the node could not stitch restarts the ring")
        assertEquals(1600, rows.fromTop)
        assertEquals("new0", readRow(rows, 0))
    }

    @Test
    fun aGapInHistoryRendersAsBlankRatherThanShiftingTheSurface() {
        val pane = pane()
        pane.applyScrollback(
            ServerMsg.Scrollback(
                pane = pane.id,
                fromTop = 0,
                rows = listOf(RowDiff(2, listOf(Run(0, "third")))),
                totalRows = 3,
                complete = false,
                capped = false,
            ),
        )
        val rows = SurfaceRows(pane)
        assertEquals(6, rows.total)
        assertEquals("", readRow(rows, 0))
        assertEquals("third", readRow(rows, 2))
        assertEquals("live0", readRow(rows, 3))
    }

    @Test
    fun theModeFallsBackWhenTheRunCacheCollapsesAndRecoversWhenItStops() {
        val modes = ModeSelector()
        repeat(20) { modes.endFrame(0.02f) }
        assertEquals(RenderMode.PerGlyph, modes.mode)

        repeat(30) {
            modes.observeRunKey(1)
            modes.observeRunKey(2)
            modes.endFrame(0f)
        }
        assertEquals(RenderMode.CachedRuns, modes.mode)
    }

    @Test
    fun aHealthyCacheNeverLeavesTheCachedRunPath() {
        val modes = ModeSelector()
        repeat(200) { modes.endFrame(0.992f) }
        assertEquals(RenderMode.CachedRuns, modes.mode)
    }
}
