package dev.kampr.terminal

import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.render.GridPoint
import dev.kampr.terminal.render.LogicalText
import dev.kampr.terminal.render.Selection
import dev.kampr.terminal.render.SurfaceRows
import dev.kampr.terminal.render.TargetKind
import dev.kampr.terminal.render.detectTarget
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

private fun paneOf(cols: Int, vararg lines: String): PaneState {
    val pane = PaneState("n/w1:p1", StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = pane.id,
            cols = cols,
            rows = lines.size,
            rowsData = lines.mapIndexed { row, text -> RowDiff(row, listOf(Run(0, text))) },
            cursor = Cursor(0, 0, true),
            links = emptyList(),
        ),
    )
    return pane
}

class SelectionTest {
    @Test
    fun copyingStripsTrailingPaddingAndKeepsRealNewlines() {
        val rows = SurfaceRows(paneOf(10, "alpha", "beta"))
        val text = LogicalText(rows).copy(Selection(GridPoint(0, 0), GridPoint(1, 9)))
        assertEquals("alpha\nbeta", text)
    }

    // A row whose last cell is not blank continued into the next one, so a path broken across the
    // grid edge has to come back as one string.
    @Test
    fun softWrappedRowsJoinWithoutANewline() {
        val rows = SurfaceRows(paneOf(10, "/home/dbra", "in/dev/kampr"))
        val text = LogicalText(rows).copy(Selection(GridPoint(0, 0), GridPoint(1, 9)))
        assertEquals("/home/dbrain/dev/kam", text)
    }

    @Test
    fun linearSelectionRunsAcrossRowsAndBlockSelectionDoesNot() {
        val cols = 6
        val linear = Selection(GridPoint(0, 3), GridPoint(2, 2))
        assertEquals(3..5, linear.span(0, cols))
        assertEquals(0..5, linear.span(1, cols))
        assertEquals(0..2, linear.span(2, cols))
        assertNull(linear.span(3, cols))

        val block = linear.copy(block = true)
        assertEquals(2..3, block.span(0, cols))
        assertEquals(2..3, block.span(1, cols))
        assertEquals(2..3, block.span(2, cols))
    }

    @Test
    fun aUrlWrappedAcrossTheGridEdgeIsStillOneTarget() {
        val rows = SurfaceRows(paneOf(20, "see https://herdr.de", "v/docs for more"))
        val logical = LogicalText(rows)
        val (line, offset) = logical.lineAt(1)
        val target = detectTarget(line, offset + 1)
        assertEquals("https://herdr.dev/docs", target?.text)
        assertEquals(TargetKind.Url, target?.kind)
    }

    @Test
    fun detectionIsAStrictSchemeMatchNotAnythingWithADot() {
        assertNull(detectTarget("edit config.toml and retry", 5))
        assertNull(detectTarget("www.example.com looks like a link", 3))
        assertEquals(TargetKind.Url, detectTarget("open http://a.b/c now", 8)?.kind)
    }

    @Test
    fun aFilePathWithALineNumberIsOfferedAsAPath() {
        val target = detectTarget("error at crates/kampr-term/src/grid.rs:42 in parse", 20)
        assertEquals("crates/kampr-term/src/grid.rs:42", target?.text)
        assertEquals(TargetKind.Path, target?.kind)
    }

    @Test
    fun anOsc8LinkIdResolvesThroughThePaneTable() {
        val pane = PaneState("n/w1:p1", StyleTable())
        pane.applyReset(
            ServerMsg.GridReset(
                pane = pane.id,
                cols = 12,
                rows = 1,
                rowsData = listOf(RowDiff(0, listOf(Run(0, "see "), Run(0, "docs", 0)))),
                cursor = Cursor(0, 0, true),
                links = listOf("https://herdr.dev"),
            ),
        )
        val logical = LogicalText(SurfaceRows(pane))
        assertEquals(0, logical.linkAt(0, 5))
        assertEquals("https://herdr.dev", pane.links.getOrNull(logical.linkAt(0, 5)))
        assertEquals(-1, logical.linkAt(0, 1))
    }
}
