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

private fun wideRow(cols: Int, vararg runs: Run): PaneState {
    val pane = PaneState("n/w1:p1", StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = pane.id,
            cols = cols,
            rows = 1,
            rowsData = listOf(RowDiff(0, runs.toList())),
            cursor = Cursor(0, 0, true),
            links = emptyList(),
        ),
    )
    return pane
}

class SelectionTest {
    // Probe #210: a wide glyph spans two columns, so a column is no longer a string index. Copy,
    // and the offset a tap resolves to, both have to be told apart from each other.
    @Test
    fun copyingAWideRowKeepsTheGlyphsAndDropsTheirTailColumns() {
        val rows = SurfaceRows(wideRow(12, Run(0, "AB"), Run(0, "\u65e5\u672c\u8a9e", w = 2), Run(0, "CD")))
        val logical = LogicalText(rows)
        assertEquals("AB\u65e5\u672c\u8a9eCD", logical.rowAt(0))
        assertEquals("\u65e5\u672c\u8a9e", logical.copy(Selection(GridPoint(0, 2), GridPoint(0, 7))))
    }

    @Test
    fun aTapPastAWideGlyphResolvesToTheCharacterUnderIt() {
        val rows = SurfaceRows(wideRow(30, Run(0, "\u65e5\u672c", w = 2), Run(0, " https://herdr.dev/x")))
        val logical = LogicalText(rows)
        val (line, offset) = logical.lineAt(0, 5)
        assertEquals("\u65e5\u672c https://herdr.dev/x", line)
        assertEquals(3, offset, "column 5 is the h of https, which is character 3")
        assertEquals("https://herdr.dev/x", detectTarget(line, offset)?.text)
    }

    // Probe #223: a cell is a grapheme, so copy has to take the marks with the base — and the
    // offset a link detector is handed is a string offset, which a mark moves and a column does not.
    @Test
    fun copyingAMarkedRowTakesTheMarksWithTheirBases() {
        val rows = SurfaceRows(wideRow(12, Run(0, "rese", m = listOf("", "\u0301", "", "\u0301"))))
        val logical = LogicalText(rows)
        assertEquals("re\u0301se\u0301", logical.rowAt(0))
        assertEquals("e\u0301s", logical.copy(Selection(GridPoint(0, 1), GridPoint(0, 2))))
    }

    @Test
    fun theOffsetOfAColumnCountsTheMarksAndTheSurrogatesBeforeIt() {
        val rows = SurfaceRows(
            wideRow(
                30,
                Run(0, "e", m = listOf("\u0301")),
                Run(0, "\uD83D\uDE80", w = 2),
                Run(0, " https://herdr.dev/x"),
            ),
        )
        val logical = LogicalText(rows)
        val (line, offset) = logical.lineAt(0, 4)
        assertEquals("e\u0301\uD83D\uDE80 https://herdr.dev/x", line)
        assertEquals(5, offset, "column 4 is the h of https, at UTF-16 offset 5")
        assertEquals("https://herdr.dev/x", detectTarget(line, offset)?.text)
    }

    // Tapping the right half of a wide glyph is tapping the glyph, not the column after it.
    @Test
    fun theTailColumnOfAWideGlyphBelongsToItsLead() {
        val rows = SurfaceRows(wideRow(12, Run(0, "ab"), Run(0, "\u65e5", w = 2), Run(0, "cd")))
        val logical = LogicalText(rows)
        assertEquals(2, logical.lineAt(0, 3).second, "the tail resolves to its lead's character")
        assertEquals("\u65e5", logical.copy(Selection(GridPoint(0, 3), GridPoint(0, 3))))
    }

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
        val (line, offset) = logical.lineAt(1, 1)
        val target = detectTarget(line, offset)
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
