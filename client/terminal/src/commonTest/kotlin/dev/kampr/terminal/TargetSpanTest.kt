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
import dev.kampr.terminal.render.detectTarget
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

private fun rowsOf(cols: Int, vararg lines: String): SurfaceRows =
    SurfaceRows(gridOf(cols, lines.size, lines.mapIndexed { row, text -> RowDiff(row, listOf(Run(0, text))) }))

private fun runRow(cols: Int, vararg runs: Run): SurfaceRows =
    SurfaceRows(gridOf(cols, 1, listOf(RowDiff(0, runs.toList()))))

private fun gridOf(cols: Int, rows: Int, data: List<RowDiff>): PaneState {
    val pane = PaneState("n/w1:p1", StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = pane.id,
            cols = cols,
            rows = rows,
            rowsData = data,
            cursor = Cursor(0, 0, true),
            links = listOf("https://herdr.dev/declared"),
        ),
    )
    return pane
}

// The cells to wash when a click hits something. `lineAt` turns a column into a string offset;
// this is the way back, and the two have to agree cell for cell or the wash sits over the wrong
// path — which on a screen holding forty of them is worse than no wash at all.
class TargetSpanTest {
    @Test
    fun aPathIsWashedOverTheCellsItWasWrittenIn() {
        val logical = LogicalText(rowsOf(60, "error: /home/dbrain/notes.md is missing"))
        val (line, offset) = logical.lineAt(0, 10)
        val target = detectTarget(line, offset)
        assertEquals("/home/dbrain/notes.md", target?.text)
        assertEquals(
            Selection(GridPoint(0, 7), GridPoint(0, 27)),
            logical.spanOf(0, target!!.range),
        )
    }

    // The compiler's `:42:9` is not part of the path the node is asked for, so it is not part of
    // what is marked as being about to be opened either.
    @Test
    fun theLineNumberAfterAPathIsNotPartOfWhatIsWashed() {
        val logical = LogicalText(rowsOf(60, "at /home/dbrain/grid.rs:42:9 in parse"))
        val (line, offset) = logical.lineAt(0, 6)
        val target = detectTarget(line, offset)
        assertEquals("/home/dbrain/grid.rs", target?.text)
        assertEquals(Selection(GridPoint(0, 3), GridPoint(0, 22)), logical.spanOf(0, target!!.range))
    }

    @Test
    fun aTargetBrokenAcrossTheGridEdgeIsWashedOnBothRows() {
        val logical = LogicalText(rowsOf(20, "see https://herdr.de", "v/docs for more"))
        val (line, offset) = logical.lineAt(1, 1)
        val target = detectTarget(line, offset)
        assertEquals("https://herdr.dev/docs", target?.text)
        assertEquals(Selection(GridPoint(0, 4), GridPoint(1, 5)), logical.spanOf(1, target!!.range))
    }

    // Probe #210: a column is not a string index once a glyph can own two of them. A wash that
    // counted characters would sit two cells left of the address on every CJK line.
    @Test
    fun wideGlyphsBeforeATargetDoNotShiftTheCellsItIsWashedOn() {
        val logical = LogicalText(runRow(40, Run(0, "日本", w = 2), Run(0, " https://herdr.dev/x")))
        val (line, offset) = logical.lineAt(0, 6)
        val target = detectTarget(line, offset)
        assertEquals("https://herdr.dev/x", target?.text)
        assertEquals(Selection(GridPoint(0, 5), GridPoint(0, 23)), logical.spanOf(0, target!!.range))
    }

    // Probe #223: a mark rides on its base's cell and costs a string offset without costing a
    // column, which is the same disagreement from the other side.
    @Test
    fun combiningMarksCostAnOffsetAndNotACell() {
        val logical = LogicalText(
            runRow(40, Run(0, "re", m = listOf("", "́")), Run(0, " /home/a.md here")),
        )
        val (line, offset) = logical.lineAt(0, 4)
        val target = detectTarget(line, offset)
        assertEquals("/home/a.md", target?.text)
        assertEquals(Selection(GridPoint(0, 3), GridPoint(0, 12)), logical.spanOf(0, target!!.range))
    }

    // A declared link has no string to find: OSC 8 puts arbitrary text on the screen and the URI
    // nowhere on it (#36/#37). So its cells are found by id, not by search.
    @Test
    fun aDeclaredLinkIsWashedOverTheCellsCarryingItsId() {
        val logical = LogicalText(
            runRow(40, Run(0, "read "), Run(0, "the guide", l = 0), Run(0, " now")),
        )
        assertEquals(Selection(GridPoint(0, 5), GridPoint(0, 13)), logical.linkSpan(0, 8))
    }

    @Test
    fun anEmptyRangeWashesNothing() {
        val logical = LogicalText(rowsOf(20, "nothing here"))
        assertNull(logical.spanOf(0, IntRange.EMPTY))
    }
}
